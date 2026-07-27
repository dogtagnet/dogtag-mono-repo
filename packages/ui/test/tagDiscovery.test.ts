// Hermetic coverage for on-chain DogTag discovery (chain/tagDiscovery.ts). No network: the chain
// reads go through an injected `TagDiscoveryReader`, the same seam `verifyCredential.test.ts` uses for
// `IssuerChainReader`. What is asserted is the ORCHESTRATION — that the scan stays bounded, cancels,
// and above all reports its coverage honestly, so a partial scan can never be presented as a complete
// one that found nothing.
//
// The tag-id derivation is checked against the REAL `@dogtag/standard` field hash (not a stub),
// because a scan that filters on the wrong id returns a confident, wrong "no activity".
// Deliberately via the BARREL, while the module under test imports `@dogtag/standard/leaf` directly
// (the barrel is browser-hostile — see the note there). Tests run in Node, where that is harmless, and
// crossing the two paths is the point: this asserts the submodule and the package's public export are
// the same function, so a subpath aimed at a stale or different dist file would fail here.
import { TypeTag, fieldOfValue } from "@dogtag/standard";
import type { Address, Hex } from "viem";
import { describe, expect, it, vi } from "vitest";
import {
  attributeSigner,
  chunkRanges,
  discoverTag,
  isOwnSigner,
  onchainDogTagId,
  resolveDogTagId,
  tagStatusLabel,
  type DiscoveredEvent,
  type TagDiscoveryReader,
} from "../src/chain/tagDiscovery";

const SBT = "0x00000000000000000000000000000000000000a1" as Address;
const VREG = "0x00000000000000000000000000000000000000a2" as Address;
const FACTORY = "0x00000000000000000000000000000000000000a3" as Address;
const CLONE = "0x00000000000000000000000000000000000000c1" as Address;
const MINTER = "0x00000000000000000000000000000000000000d1" as Address;
const OTHER_SHOP = "0x00000000000000000000000000000000000000e1" as Address;
const ZERO = "0x0000000000000000000000000000000000000000" as Address;
const ADDRESSES = { sbt: SBT, verificationRegistry: VREG, factory: FACTORY };

const ROOT = `0x${"ab".repeat(32)}` as Hex;
const ZERO_HASH = `0x${"0".repeat(64)}` as Hex;

function txHash(n: number): Hex {
  return `0x${n.toString(16).padStart(64, "0")}` as Hex;
}

/** A verification log at `blockNumber`, by default from a relayer that is NOT this shop. */
function verification(blockNumber: bigint, relayer: Address = OTHER_SHOP): DiscoveredEvent {
  return {
    kind: "verification",
    blockNumber,
    txHash: txHash(Number(blockNumber)),
    contract: VREG,
    relayer,
    purposeKey: `0x${"11".repeat(32)}` as Hex,
    nullifier: `0x${"22".repeat(32)}` as Hex,
    deadline: 1_800_000_000n,
    ts: 1_700_000_000n,
  };
}

interface ReaderCfg {
  latest?: bigint;
  profileRoot?: Hex;
  issuerClone?: Address;
  issuedAt?: bigint;
  revoked?: boolean;
  valid?: boolean;
  status?: number;
  /** Events keyed by the chunk's `fromBlock`, so a test can place a log in a specific chunk. */
  logs?: DiscoveredEvent[];
  /** `fromBlock` values whose chunk request must reject. */
  failRanges?: bigint[];
  /** Called with each requested range, so the test can assert boundedness. */
  onRange?: (from: bigint, to: bigint) => void;
}

function fakeReader(cfg: ReaderCfg = {}): TagDiscoveryReader {
  const logs = cfg.logs ?? [];
  return {
    latestBlock: async () => cfg.latest ?? 1_000n,
    profileRoot: async () => cfg.profileRoot ?? ROOT,
    issuerOf: async () => MINTER,
    statusOf: async () => cfg.status ?? 0,
    rootIssuer: async () => cfg.issuerClone ?? CLONE,
    anchorState: async () => ({
      issuedAt: cfg.issuedAt ?? 1_700_000_000n,
      revoked: cfg.revoked ?? false,
      valid: cfg.valid ?? true,
    }),
    tagLogs: async ({ fromBlock, toBlock }) => {
      cfg.onRange?.(fromBlock, toBlock);
      if (cfg.failRanges?.some((f) => f === fromBlock)) {
        throw new Error(`rpc: range ${fromBlock}-${toBlock} exceeded`);
      }
      return logs.filter((e) => e.blockNumber >= fromBlock && e.blockNumber <= toBlock);
    },
  };
}

describe("dogTagId resolution", () => {
  it("field-hashes an operator handle to the id the chain is keyed by", () => {
    // The chain indexes `fieldOfValue(Integer(handle))`, never the handle itself.
    expect(onchainDogTagId("4")).toBe(fieldOfValue({ tag: TypeTag.Integer, value: "4" }));
    expect(onchainDogTagId("4")).not.toBe(4n);

    const r = resolveDogTagId("4");
    expect(r).not.toBeNull();
    expect(r?.form).toBe("handle");
    expect(r?.handle).toBe("4");
    expect(r?.onchain).toBe(onchainDogTagId("4"));
  });

  it("accepts a field element already in decimal or hex form, unchanged", () => {
    const field = onchainDogTagId("4");
    const asDecimal = resolveDogTagId(field.toString());
    expect(asDecimal?.form).toBe("field");
    expect(asDecimal?.onchain).toBe(field);
    expect(asDecimal?.handle).toBeNull();

    const asHex = resolveDogTagId(`0x${field.toString(16)}`);
    expect(asHex?.form).toBe("field");
    expect(asHex?.onchain).toBe(field);
  });

  it("refuses anything that is not a usable id rather than scanning tag 0", () => {
    // Each of these previously would have become a scan whose empty result looked authoritative.
    for (const bad of ["", "   ", "not-a-tag", "12ab", "0x", "0xzz", undefined, null]) {
      expect(resolveDogTagId(bad as string | null | undefined)).toBeNull();
    }
  });

  it("names every status the contract's enum defines, in its declaration order", () => {
    // Transcribed from `contracts/src/DogTagSBTConsent.sol`:
    //   enum Status { Active, Lost, TransferPending, Deceased, Revoked }
    // The ORDER is the meaning, so a short or reordered list does not degrade to "unknown" - it
    // renames one real status as another. A TransferPending tag announced as "revoked", or a
    // genuinely revoked one as "unknown", is exactly the confident-wrong claim this module exists to
    // prevent, so assert all five rather than a prefix.
    expect(tagStatusLabel(0)).toBe("active");
    expect(tagStatusLabel(1)).toBe("lost");
    expect(tagStatusLabel(2)).toBe("transfer pending");
    expect(tagStatusLabel(3)).toBe("deceased");
    expect(tagStatusLabel(4)).toBe("revoked");
    // A byte the enum does not define must not be given an invented meaning.
    expect(tagStatusLabel(5)).toBe("unknown");
    expect(tagStatusLabel(7)).toBe("unknown");
  });
});

describe("chunkRanges", () => {
  it("covers the window exactly, with an inclusive final partial chunk", () => {
    expect(chunkRanges(0n, 9n, 4n)).toEqual([
      { fromBlock: 0n, toBlock: 3n },
      { fromBlock: 4n, toBlock: 7n },
      { fromBlock: 8n, toBlock: 9n },
    ]);
  });

  it("is empty when the window is", () => {
    expect(chunkRanges(10n, 9n, 4n)).toEqual([]);
  });
});

describe("discoverTag", () => {
  it("bounds the scan to the lookback window and never scans from genesis", async () => {
    const ranges: Array<[bigint, bigint]> = [];
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({ latest: 500_000n, onRange: (f, t) => ranges.push([f, t]) }),
      lookbackBlocks: 100n,
      chunkBlocks: 25n,
    });
    expect(r.coverage.fromBlock).toBe(499_901n);
    expect(r.coverage.toBlock).toBe(500_000n);
    // A complete scan reached the window's floor, so the two agree — which is the ONLY case in which
    // reporting `fromBlock` as read is truthful.
    expect(r.coverage.reachedBlock).toBe(499_901n);
    expect(r.coverage.chunksTotal).toBe(4);
    expect(ranges).toHaveLength(4);
    // Nothing outside the declared window is ever requested.
    expect(ranges.every(([f, t]) => f >= 499_901n && t <= 500_000n)).toBe(true);
    expect(r.coverage.complete).toBe(true);
  });

  it("clamps the window at block 0 on a chain younger than the lookback", async () => {
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({ latest: 10n }),
      lookbackBlocks: 100_000n,
      chunkBlocks: 25n,
    });
    expect(r.coverage.fromBlock).toBe(0n);
    expect(r.coverage.toBlock).toBe(10n);
  });

  it("resolves the tag's profile credential through the chain and reports the events found", async () => {
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({
        latest: 100n,
        logs: [
          verification(90n),
          { kind: "mint", blockNumber: 50n, txHash: txHash(50), contract: SBT, issuer: MINTER },
        ],
      }),
      lookbackBlocks: 100n,
      chunkBlocks: 50n,
    });
    expect(r.profile).toEqual({
      root: ROOT,
      issuerClone: CLONE,
      issuedAt: 1_700_000_000n,
      revoked: false,
      valid: true,
    });
    expect(r.mintedBy).toBe(MINTER);
    expect(r.statusLabel).toBe("active");
    // Newest first, so the most recent activity is what an operator sees first.
    expect(r.events.map((e) => e.blockNumber)).toEqual([90n, 50n]);
    expect(r.coverage.complete).toBe(true);
  });

  it("reports no profile credential when the chain holds no tag under the id", async () => {
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({ latest: 100n, profileRoot: ZERO_HASH }),
      lookbackBlocks: 100n,
      chunkBlocks: 100n,
    });
    // A COMPLETE scan that genuinely found nothing — distinguishable from a failed one below.
    expect(r.profile).toBeNull();
    expect(r.events).toEqual([]);
    expect(r.coverage.complete).toBe(true);
    expect(r.coverage.failures).toEqual([]);
  });

  it("keeps a root the factory cannot resolve, with a null clone, rather than dropping it", async () => {
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({ latest: 10n, issuerClone: ZERO }),
      lookbackBlocks: 10n,
      chunkBlocks: 10n,
    });
    // The SBT says this root exists; claiming otherwise would contradict a read already made.
    expect(r.profile?.root).toBe(ROOT);
    expect(r.profile?.issuerClone).toBeNull();
    expect(r.profile?.valid).toBe(false);
    expect(r.profile?.issuedAt).toBe(0n);
  });

  it("records a failed chunk as UNCOVERED instead of reporting a clean empty result", async () => {
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      // The window is 0..99 in four chunks; the 25..49 chunk fails.
      reader: fakeReader({ latest: 99n, failRanges: [25n], logs: [] }),
      lookbackBlocks: 100n,
      chunkBlocks: 25n,
    });
    expect(r.coverage.complete).toBe(false);
    expect(r.coverage.failures).toHaveLength(1);
    expect(r.coverage.failures[0].fromBlock).toBe(25n);
    expect(r.coverage.failures[0].toBlock).toBe(49n);
    expect(r.coverage.failures[0].message).toContain("rpc:");
    // The other three chunks still ran — a partial answer is kept, it is just not called complete.
    expect(r.coverage.chunksDone).toBe(4);
  });

  it("keeps the events from chunks that DID succeed when another fails", async () => {
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({ latest: 99n, failRanges: [25n], logs: [verification(80n)] }),
      lookbackBlocks: 100n,
      chunkBlocks: 25n,
    });
    expect(r.events).toHaveLength(1);
    expect(r.coverage.complete).toBe(false);
  });

  it("cancels between chunks and marks the partial result cancelled", async () => {
    const ctrl = new AbortController();
    let seen = 0;
    const reader = fakeReader({
      latest: 999n,
      logs: [],
      onRange: () => {
        // Abort after the first chunk; the scan must stop rather than run the remaining 9.
        if (++seen === 1) ctrl.abort();
      },
    });
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader,
      lookbackBlocks: 1_000n,
      chunkBlocks: 100n,
      signal: ctrl.signal,
    });
    expect(r.coverage.chunksTotal).toBe(10);
    expect(seen).toBe(1);
    expect(r.coverage.chunksDone).toBe(1);
    expect(r.coverage.cancelled).toBe(true);
    // Cancelled is never complete: the operator stopped it, so most of the window is unknown.
    expect(r.coverage.complete).toBe(false);
    // Only the newest chunk was read, so the reported extent must be that chunk's floor (900) — NOT
    // the requested window's floor (0), which nothing looked at.
    expect(r.coverage.fromBlock).toBe(0n);
    expect(r.coverage.reachedBlock).toBe(900n);
  });

  it("does not start any chunk when already aborted", async () => {
    const ctrl = new AbortController();
    ctrl.abort();
    const onRange = vi.fn();
    const r = await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({ latest: 999n, onRange }),
      lookbackBlocks: 1_000n,
      chunkBlocks: 100n,
      signal: ctrl.signal,
    });
    expect(onRange).not.toHaveBeenCalled();
    expect(r.coverage.cancelled).toBe(true);
    expect(r.coverage.complete).toBe(false);
    // Nothing was read at all, so there is no extent to report — the panel must say "no blocks were
    // read" rather than print the requested window as if it had been covered.
    expect(r.coverage.reachedBlock).toBeNull();
    expect(r.coverage.chunksDone).toBe(0);
  });

  it("reports progress per chunk, newest range first", async () => {
    const progress: Array<{ done: number; reachedBlock: bigint; found: number }> = [];
    await discoverTag({
      dogTagId: 42n,
      addresses: ADDRESSES,
      reader: fakeReader({ latest: 99n, logs: [verification(95n)] }),
      lookbackBlocks: 100n,
      chunkBlocks: 25n,
      onProgress: (p) =>
        progress.push({ done: p.chunksDone, reachedBlock: p.reachedBlock, found: p.found }),
    });
    expect(progress.map((p) => p.done)).toEqual([1, 2, 3, 4]);
    // Head-first, so `reachedBlock` moves DOWN: the newest chunk (75..99) is scanned first, its hit
    // appears immediately, and what has been covered is always [reachedBlock, toBlock].
    expect(progress[0].reachedBlock).toBe(75n);
    expect(progress[0].found).toBe(1);
    expect(progress[3].reachedBlock).toBe(0n);
  });

  it("rejects when a point-in-time read fails, rather than resolving an empty result", async () => {
    const reader = fakeReader({ latest: 100n });
    reader.profileRoot = async () => {
      throw new Error("rpc down");
    };
    // There is no result to qualify without these reads, so an empty one would be a fabrication.
    await expect(
      discoverTag({ dogTagId: 42n, addresses: ADDRESSES, reader, lookbackBlocks: 10n }),
    ).rejects.toThrow("rpc down");
  });

  it("rejects a non-positive chunk size instead of looping forever", async () => {
    await expect(
      discoverTag({
        dogTagId: 42n,
        addresses: ADDRESSES,
        reader: fakeReader(),
        chunkBlocks: 0n,
      }),
    ).rejects.toThrow("chunkBlocks must be positive");
  });
});

describe("isOwnSigner", () => {
  it("matches case-insensitively so a checksummed address is still recognised as ours", () => {
    expect(isOwnSigner("0xABCDEF0000000000000000000000000000000001", ["0xabcdef0000000000000000000000000000000001"])).toBe(true);
  });

  it("treats an unknown relayer as NOT ours — the finding that drives the import handoff", () => {
    expect(isOwnSigner(OTHER_SHOP, [MINTER])).toBe(false);
  });

  it("is a membership predicate only, so an empty set makes every address a non-member", () => {
    // Correct as an ANSWER, and unusable as an attribution: "not in this set" and "we have no set"
    // are the same boolean here, which is why nothing user-facing may call this directly. The
    // attribution a reader is shown goes through `attributeSigner`, asserted below.
    expect(isOwnSigner(MINTER, [])).toBe(false);
    expect(isOwnSigner(null, [MINTER])).toBe(false);
  });
});

describe("attributeSigner", () => {
  it("names ours and a stranger's apart when the signer set is known", () => {
    expect(attributeSigner(MINTER, [MINTER])).toBe("own");
    expect(attributeSigner(OTHER_SHOP, [MINTER])).toBe("other");
  });

  it("reports UNKNOWN rather than 'other' when the signer set could not be established", () => {
    // The defect this exists to close: `GET /issuer/signers` answers 200 `{"signers": []}` whenever
    // custody is LOCKED, so folding that into "not ours" would turn missing data into a positive
    // claim that a stranger verified this pet - complete with an ask-the-owner CTA driven off it.
    expect(attributeSigner(MINTER, null)).toBe("unknown");
    expect(attributeSigner(OTHER_SHOP, null)).toBe("unknown");
  });

  it("treats a KNOWN-but-empty set as unknown too - there is nothing to compare against", () => {
    expect(attributeSigner(MINTER, [])).toBe("unknown");
  });

  it("reports an absent actor as unknown, never as a stranger", () => {
    expect(attributeSigner(null, [MINTER])).toBe("unknown");
    expect(attributeSigner(undefined, [MINTER])).toBe("unknown");
  });
});
