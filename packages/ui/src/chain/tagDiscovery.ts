/**
 * ON-CHAIN DISCOVERY for one DogTag: what exists on chain against this tag, including what this shop
 * never saw.
 *
 * The problem this solves is the returning owner. A pet arrives at a shop that has no record of it;
 * the tag itself is the only handle anyone has. Everything the shop knows lives in its own database,
 * so without a chain read the operator's only options are to ask the owner to produce a credential
 * out of thin air, or to treat the pet as brand new. This module reads the chain directly and reports
 * what is genuinely there.
 *
 * ## What is discoverable, and what is NOT
 *
 * This distinction is the whole design, and it is a privacy property rather than a limitation to be
 * worked around:
 *
 *  - **Discoverable.** The tag's Level-A facts are *keyed by* `dogTagId` on chain, and the owner-blind
 *    `Verified` event **indexes** it. So a tag id yields: its profile root, the issuer that minted it,
 *    its status, its mint/status/burn events, the anchoring state of its profile credential in the
 *    `DogTagIssuer` clone that holds it — and every verification ever recorded against it, by ANY
 *    relayer, whether or not this shop was involved.
 *  - **NOT discoverable.** Level-B credential roots are deliberately **not bound to a `dogTagId` on
 *    chain**. `DogTagIssuer.RootIssued` carries the root and nothing else, and the consent circuit
 *    binds `dogTagId <-> R` only for the tag's own profile root. This is intentional: if a tag id
 *    enumerated every credential ever issued to that animal, the tag would be a public dossier
 *    lookup key. A scan therefore CANNOT list "all of this pet's vaccination records", and this
 *    module never pretends otherwise.
 *
 * What makes that second half useful rather than merely honest: a `Verified` event from a relayer this
 * shop does not recognise is positive evidence that credentials exist which the shop has never seen.
 * That is precisely the cue to ask the owner to share them — the import handoff — and it is a real
 * finding read off the chain, not an inference dressed up as one.
 *
 * ## Bounded, cancellable, and honest about coverage
 *
 * Log scans are chunked over a bounded block window, report progress per chunk, and abort promptly.
 * A chunk that FAILS is recorded rather than swallowed: the scan reports the range it actually
 * covered, so "completed and found nothing" and "could not finish" can never be confused. Silently
 * eating a failed chunk would render a partial scan as a clean empty result, which is the same defect
 * as rendering an unchecked credential as valid.
 *
 * Nothing here writes: every call is an `eth_call` or `eth_getLogs`. No signer, no gas, no auth.
 */

// Imported from the SUBMODULES, not the `@dogtag/standard` barrel. The barrel re-exports
// `consent.ts`, whose `circomlibjs` EdDSA/BabyJubjub dependency touches the Node `Buffer` global at
// module init; in a browser that is a `ReferenceError` which takes down whatever imported it. A
// production bundler tree-shakes the unused import away, so the crash appears only under a dev server
// — the worst kind of latent break. Nothing here needs EdDSA: `fieldOfValue` is Poseidon-only, so the
// fix is to depend on exactly the two leaf modules this uses.
import { fieldOfValue } from "@dogtag/standard/leaf";
import { TypeTag } from "@dogtag/standard/types";
import type { Abi, Address, Hex } from "viem";
import { DEPLOYED_ADDRESSES, roaxPublicClient } from "../wallet/contracts";

// --------------------------------------------------------------------------------------------
// the tag id: what the operator types vs. what the chain is keyed by
// --------------------------------------------------------------------------------------------

/**
 * The on-chain `dogTagId` is `fieldOfValue(Integer(handle))` — a BN254 field element — while the
 * OPERATOR-facing id is the short decimal handle (`"4"`). The contracts, the circuit's `pub[0]` and
 * every indexed event topic use the field element; `DogTagSBTConsent.profileRoot` is keyed by it.
 *
 * A scan must therefore never use the handle as a topic filter. It would be a well-formed uint256
 * that simply matches nothing, and the panel would render a confident, wrong "no on-chain activity"
 * for a tag with plenty of it. Mirrors vet-api `onchain_dog_tag_id` and the `field-hash` binary.
 */
export function onchainDogTagId(handle: string): bigint {
  return fieldOfValue({ tag: TypeTag.Integer, value: handle.trim() });
}

/** How a stored `dogTagId` string was interpreted, so the UI can show the operator which it used. */
export interface ResolvedDogTagId {
  /** The uint256 the chain is keyed by. */
  onchain: bigint;
  /** The operator-facing handle, when the stored value was one. */
  handle: string | null;
  /**
   * `handle` - a short decimal that was field-hashed to reach the chain key.
   * `field`  - the stored value already WAS the field element (decimal or 0x-hex), used as-is.
   */
  form: "handle" | "field";
}

/**
 * Beyond this many decimal digits a value cannot be an operator-facing handle: handles come from a
 * monotonic counter (`Store::next_dog_tag_id`), so 21 digits is already past `u64`. A field element,
 * by contrast, is up to 77 digits. The split is what lets an operator paste EITHER the short handle
 * their portal shows or the long id an explorer/wallet shows, and have both resolve to the same tag.
 */
const MAX_HANDLE_DIGITS = 20;

/**
 * Resolve whatever the shop stored on the pet into the uint256 the chain is keyed by.
 *
 * Returns `null` for anything that is not a usable id, so a caller can never scan on a typo: a blank,
 * a non-numeric string, or a hex value of the wrong width yields no scan and a visible reason, rather
 * than a scan of tag `0` reported as an empty result.
 */
export function resolveDogTagId(stored?: string | null): ResolvedDogTagId | null {
  const s = (stored ?? "").trim();
  if (!s) return null;
  if (/^0x[0-9a-fA-F]{1,64}$/.test(s)) {
    // Already the field element, in the hex form an explorer or a wrapped credential shows.
    return { onchain: BigInt(s), handle: null, form: "field" };
  }
  if (!/^\d+$/.test(s)) return null;
  if (s.replace(/^0+/, "").length > MAX_HANDLE_DIGITS) {
    return { onchain: BigInt(s), handle: null, form: "field" };
  }
  return { onchain: onchainDogTagId(s), handle: s, form: "handle" };
}

// --------------------------------------------------------------------------------------------
// the reads
// --------------------------------------------------------------------------------------------

const SBT_ABI = [
  {
    type: "function",
    name: "profileRoot",
    stateMutability: "view",
    inputs: [{ name: "id", type: "uint256" }],
    outputs: [{ name: "", type: "bytes32" }],
  },
  {
    type: "function",
    name: "issuerOf",
    stateMutability: "view",
    inputs: [{ name: "id", type: "uint256" }],
    outputs: [{ name: "", type: "address" }],
  },
  {
    type: "function",
    name: "status",
    stateMutability: "view",
    inputs: [{ name: "id", type: "uint256" }],
    outputs: [{ name: "", type: "uint8" }],
  },
  {
    type: "event",
    name: "Issued",
    inputs: [
      { name: "dogTagId", type: "uint256", indexed: true },
      { name: "issuer", type: "address", indexed: true },
    ],
  },
  {
    type: "event",
    name: "StatusChanged",
    inputs: [
      { name: "dogTagId", type: "uint256", indexed: true },
      { name: "from", type: "uint8", indexed: false },
      { name: "to", type: "uint8", indexed: false },
      { name: "by", type: "address", indexed: false },
      { name: "reason", type: "string", indexed: false },
    ],
  },
  {
    type: "event",
    name: "Burned",
    inputs: [{ name: "dogTagId", type: "uint256", indexed: true }],
  },
] as const satisfies Abi;

const VERIFICATION_REGISTRY_ABI = [
  {
    type: "event",
    name: "Verified",
    inputs: [
      { name: "dogTagId", type: "uint256", indexed: true },
      { name: "relayer", type: "address", indexed: true },
      { name: "purpose", type: "bytes32", indexed: false },
      { name: "nullifier", type: "bytes32", indexed: false },
      { name: "deadline", type: "uint256", indexed: false },
      { name: "ts", type: "uint256", indexed: false },
    ],
  },
] as const satisfies Abi;

const FACTORY_ABI = [
  {
    type: "function",
    name: "rootIssuer",
    stateMutability: "view",
    inputs: [{ name: "root", type: "bytes32" }],
    outputs: [{ name: "", type: "address" }],
  },
] as const satisfies Abi;

const ISSUER_ABI = [
  {
    type: "function",
    name: "issuedAt",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    type: "function",
    name: "isRevoked",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "bool" }],
  },
  {
    type: "function",
    name: "isValid",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "bool" }],
  },
] as const satisfies Abi;

/** The `DogTagSBTConsent.Status` enum, in declaration order. */
export const TAG_STATUS_LABELS = ["active", "suspended", "revoked"] as const;
export type TagStatusLabel = (typeof TAG_STATUS_LABELS)[number] | "unknown";

/** Name a raw `status` byte, without inventing a meaning for a value the enum does not define. */
export function tagStatusLabel(status: number): TagStatusLabel {
  return TAG_STATUS_LABELS[status] ?? "unknown";
}

/** One log the scan turned up, reduced to the fields a row needs. */
export interface DiscoveredEventBase {
  blockNumber: bigint;
  txHash: Hex;
  /** Emitting contract, so a row can say WHICH contract it came from. */
  contract: Address;
}

export interface DiscoveredMint extends DiscoveredEventBase {
  kind: "mint";
  issuer: Address;
}

export interface DiscoveredStatusChange extends DiscoveredEventBase {
  kind: "statusChange";
  from: number;
  to: number;
  by: Address;
  reason: string;
}

export interface DiscoveredBurn extends DiscoveredEventBase {
  kind: "burn";
}

export interface DiscoveredVerification extends DiscoveredEventBase {
  kind: "verification";
  relayer: Address;
  /** `keccak256(label) % r` — a HASHED key. Never renderable as a purpose word on its own. */
  purposeKey: Hex;
  nullifier: Hex;
  deadline: bigint;
  /** The contract's own `block.timestamp` at the time of recording, in unix SECONDS. */
  ts: bigint;
}

export type DiscoveredEvent =
  | DiscoveredMint
  | DiscoveredStatusChange
  | DiscoveredBurn
  | DiscoveredVerification;

/**
 * The tag's Level-A profile credential, resolved through the chain rather than through any database:
 * `profileRoot(dogTagId)` -> `rootIssuer(root)` -> that clone's anchoring state.
 *
 * For a shop meeting a pet for the first time this is a credential it did not create, discovered from
 * the tag alone.
 */
export interface DiscoveredProfileCredential {
  root: Hex;
  /** The `DogTagIssuer` clone the root is anchored in, or `null` when the factory knows no issuer. */
  issuerClone: Address | null;
  /** Anchoring time in unix seconds; `0n` means the root was never anchored. */
  issuedAt: bigint;
  revoked: boolean;
  /** `issuedAt != 0 && !revoked`, as the clone itself computes it. */
  valid: boolean;
}

/** What the scan covered, so a reader can tell a complete empty result from an incomplete one. */
export interface DiscoveryCoverage {
  /** Chain head at the moment the scan started. */
  latestBlock: bigint;
  /** First block of the REQUESTED window — not a claim that it was read. See `reachedBlock`. */
  fromBlock: bigint;
  /** Last block of the requested window (the head the scan started from). */
  toBlock: bigint;
  /**
   * The lowest block the scan actually got to, or `null` when no chunk completed at all.
   *
   * Separate from `fromBlock` because the two diverge exactly when it matters. The scan works
   * head-first, so a cancelled or half-failed run has covered `[reachedBlock, toBlock]` and knows
   * nothing below it — reporting the requested `fromBlock` as if it had been read is the overstatement
   * this field exists to prevent. It is an EXTENT, not a guarantee of completeness within it:
   * `failures` names any range inside it that could not be read.
   */
  reachedBlock: bigint | null;
  chunksTotal: number;
  chunksDone: number;
  /** One entry per chunk whose `eth_getLogs` failed, naming the range that is therefore UNKNOWN. */
  failures: Array<{ fromBlock: bigint; toBlock: bigint; message: string }>;
  /** True only when every chunk of the requested window succeeded. */
  complete: boolean;
  /** Set when the operator cancelled; the coverage above is then what was reached before stopping. */
  cancelled: boolean;
}

export interface TagDiscoveryResult {
  dogTagId: bigint;
  /** `sbt.status(dogTagId)`, raw, plus its label. */
  status: number;
  statusLabel: TagStatusLabel;
  /** `sbt.issuerOf(dogTagId)` — the issuer that minted the tag. Zero address when never minted. */
  mintedBy: Address;
  /**
   * `null` when `profileRoot(dogTagId)` is zero, i.e. the chain holds no tag under this id. That is a
   * genuine and important finding, distinct from a scan that failed.
   */
  profile: DiscoveredProfileCredential | null;
  events: DiscoveredEvent[];
  coverage: DiscoveryCoverage;
}

/**
 * The reads discovery needs, behind an interface so the orchestration — chunking, cancellation,
 * partial-failure accounting — is testable without a chain. The default implementation is
 * {@link roaxTagDiscoveryReader}: plain viem against the public ROAX RPC.
 */
export interface TagDiscoveryReader {
  latestBlock(): Promise<bigint>;
  profileRoot(sbt: Address, dogTagId: bigint): Promise<Hex>;
  issuerOf(sbt: Address, dogTagId: bigint): Promise<Address>;
  statusOf(sbt: Address, dogTagId: bigint): Promise<number>;
  rootIssuer(factory: Address, root: Hex): Promise<Address>;
  anchorState(clone: Address, root: Hex): Promise<{ issuedAt: bigint; revoked: boolean; valid: boolean }>;
  /**
   * Logs for ONE bounded block range. Rejecting is meaningful and must not be swallowed by the
   * implementation: the orchestrator records the failed range so coverage stays honest.
   */
  tagLogs(args: {
    sbt: Address;
    verificationRegistry: Address;
    dogTagId: bigint;
    fromBlock: bigint;
    toBlock: bigint;
  }): Promise<DiscoveredEvent[]>;
}

export interface DiscoveryAddresses {
  /** `DogTagSBTConsent` — the tag itself: profileRoot/issuerOf/status + Issued/StatusChanged/Burned. */
  sbt: Address;
  /** `VerificationRegistryConsent` — the owner-blind `Verified` events, indexed by dogTagId. */
  verificationRegistry: Address;
  /** `DogTagIssuerFactory` — `rootIssuer(root)`, which resolves a root to the clone holding it. */
  factory: Address;
}

/**
 * Default addresses for a ROAX deployment. Note the VERIFICATION REGISTRY: discovery must read the
 * owner-hidden `VerificationRegistryConsent`, whose `Verified` event indexes `dogTagId`. The older
 * `VerificationRegistry` in {@link DEPLOYED_ADDRESSES} is a different, superseded contract; pointing a
 * scan at it returns zero events and looks exactly like an honestly empty chain.
 */
export const DISCOVERY_ADDRESSES: DiscoveryAddresses = {
  sbt: DEPLOYED_ADDRESSES.DogTagSBT as Address,
  verificationRegistry: DEPLOYED_ADDRESSES.VerificationRegistryConsent as Address,
  factory: DEPLOYED_ADDRESSES.DogTagIssuerFactory as Address,
};

const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000" as Address;
const ZERO_HASH32 = `0x${"0".repeat(64)}` as Hex;

/** Default reader: viem `eth_call`/`eth_getLogs` against the public ROAX RPC. No signer, no auth. */
export function roaxTagDiscoveryReader(rpcUrl?: string): TagDiscoveryReader {
  const client = () => roaxPublicClient(rpcUrl);
  return {
    latestBlock: () => client().getBlockNumber(),
    profileRoot: (sbt, dogTagId) =>
      client().readContract({
        address: sbt,
        abi: SBT_ABI,
        functionName: "profileRoot",
        args: [dogTagId],
      }) as Promise<Hex>,
    issuerOf: (sbt, dogTagId) =>
      client().readContract({
        address: sbt,
        abi: SBT_ABI,
        functionName: "issuerOf",
        args: [dogTagId],
      }) as Promise<Address>,
    statusOf: async (sbt, dogTagId) =>
      Number(
        await client().readContract({
          address: sbt,
          abi: SBT_ABI,
          functionName: "status",
          args: [dogTagId],
        }),
      ),
    rootIssuer: (factory, root) =>
      client().readContract({
        address: factory,
        abi: FACTORY_ABI,
        functionName: "rootIssuer",
        args: [root],
      }) as Promise<Address>,
    anchorState: async (clone, root) => {
      const [issuedAt, revoked, valid] = await Promise.all([
        client().readContract({
          address: clone,
          abi: ISSUER_ABI,
          functionName: "issuedAt",
          args: [root],
        }) as Promise<bigint>,
        client().readContract({
          address: clone,
          abi: ISSUER_ABI,
          functionName: "isRevoked",
          args: [root],
        }) as Promise<boolean>,
        client().readContract({
          address: clone,
          abi: ISSUER_ABI,
          functionName: "isValid",
          args: [root],
        }) as Promise<boolean>,
      ]);
      return { issuedAt, revoked, valid };
    },
    tagLogs: async ({ sbt, verificationRegistry, dogTagId, fromBlock, toBlock }) => {
      const c = client();
      // All four events index `dogTagId` first, so the NODE does the filtering: one topic-filtered
      // request per contract per chunk, never a fetch-everything-then-filter-locally scan.
      const [issued, statusChanged, burned, verified] = await Promise.all([
        c.getLogs({
          address: sbt,
          event: SBT_ABI[3],
          args: { dogTagId },
          fromBlock,
          toBlock,
        }),
        c.getLogs({
          address: sbt,
          event: SBT_ABI[4],
          args: { dogTagId },
          fromBlock,
          toBlock,
        }),
        c.getLogs({
          address: sbt,
          event: SBT_ABI[5],
          args: { dogTagId },
          fromBlock,
          toBlock,
        }),
        c.getLogs({
          address: verificationRegistry,
          event: VERIFICATION_REGISTRY_ABI[0],
          args: { dogTagId },
          fromBlock,
          toBlock,
        }),
      ]);
      const out: DiscoveredEvent[] = [];
      for (const l of issued) {
        out.push({
          kind: "mint",
          blockNumber: l.blockNumber,
          txHash: l.transactionHash,
          contract: l.address,
          issuer: l.args.issuer as Address,
        });
      }
      for (const l of statusChanged) {
        out.push({
          kind: "statusChange",
          blockNumber: l.blockNumber,
          txHash: l.transactionHash,
          contract: l.address,
          from: Number(l.args.from),
          to: Number(l.args.to),
          by: l.args.by as Address,
          reason: (l.args.reason as string) ?? "",
        });
      }
      for (const l of burned) {
        out.push({
          kind: "burn",
          blockNumber: l.blockNumber,
          txHash: l.transactionHash,
          contract: l.address,
        });
      }
      for (const l of verified) {
        out.push({
          kind: "verification",
          blockNumber: l.blockNumber,
          txHash: l.transactionHash,
          contract: l.address,
          relayer: l.args.relayer as Address,
          purposeKey: l.args.purpose as Hex,
          nullifier: l.args.nullifier as Hex,
          deadline: l.args.deadline as bigint,
          ts: l.args.ts as bigint,
        });
      }
      return out;
    },
  };
}

/** Progress for the operator: which chunk, over what range, and what has been found so far. */
export interface DiscoveryProgress {
  chunksDone: number;
  chunksTotal: number;
  /** Highest block confirmed scanned so far. */
  scannedTo: bigint;
  fromBlock: bigint;
  toBlock: bigint;
  /** Events found so far — a long scan shows results as they arrive rather than only at the end. */
  found: number;
  failures: number;
}

export interface DiscoverTagArgs {
  /** The tag's ON-CHAIN id. Use {@link resolveDogTagId} on whatever the shop stored. */
  dogTagId: bigint;
  addresses?: DiscoveryAddresses;
  rpcUrl?: string;
  reader?: TagDiscoveryReader;
  /**
   * How far back to scan, in blocks, from the chain head. Bounded by construction: an unbounded
   * `fromBlock: 0` scan is what makes a portal hang on a chain of any age.
   */
  lookbackBlocks?: bigint;
  /** Blocks per `eth_getLogs` request. Smaller chunks mean finer progress and smaller retries. */
  chunkBlocks?: bigint;
  onProgress?: (p: DiscoveryProgress) => void;
  /** Cancels promptly, between chunks. The partial result is returned with `cancelled: true`. */
  signal?: AbortSignal;
}

/**
 * How far back a scan reaches by default. ROAX is a young chain and the demo deployment's whole
 * history is well inside this, so the default window covers every tag currently in existence while
 * still being a BOUND rather than a full-history scan.
 */
export const DEFAULT_LOOKBACK_BLOCKS = 400_000n;
/**
 * Blocks per request. Chosen so a full default window is a few dozen requests: enough chunks for
 * progress to move visibly and for one failure to cost a small, named range rather than the scan.
 */
export const DEFAULT_CHUNK_BLOCKS = 20_000n;

/**
 * Scan the chain for everything recorded against one DogTag.
 *
 * Resolves even when chunks failed or the operator cancelled — the outcome is in
 * {@link DiscoveryCoverage}, because a discovery surface has to be able to say "here is what I could
 * and could not see". It rejects only when the *point-in-time* reads fail (head, profileRoot,
 * issuerOf, status): without those there is no result to qualify, and reporting an empty one would be
 * a fabrication.
 */
export async function discoverTag(args: DiscoverTagArgs): Promise<TagDiscoveryResult> {
  const addresses = args.addresses ?? DISCOVERY_ADDRESSES;
  const reader = args.reader ?? roaxTagDiscoveryReader(args.rpcUrl);
  const lookback = args.lookbackBlocks ?? DEFAULT_LOOKBACK_BLOCKS;
  const chunk = args.chunkBlocks ?? DEFAULT_CHUNK_BLOCKS;
  if (chunk <= 0n) throw new Error("chunkBlocks must be positive");
  const { dogTagId } = args;

  const latestBlock = await reader.latestBlock();
  const fromBlock = latestBlock >= lookback ? latestBlock - lookback + 1n : 0n;
  const toBlock = latestBlock;

  // Point-in-time state first. These are single `eth_call`s at the head, and they answer the question
  // a scan cannot: whether the chain holds a tag under this id AT ALL.
  const [profileRootValue, mintedBy, status] = await Promise.all([
    reader.profileRoot(addresses.sbt, dogTagId),
    reader.issuerOf(addresses.sbt, dogTagId),
    reader.statusOf(addresses.sbt, dogTagId),
  ]);

  let profile: DiscoveredProfileCredential | null = null;
  if (profileRootValue && profileRootValue !== ZERO_HASH32) {
    // Resolve the credential through the chain: root -> issuing clone -> that clone's anchor state.
    // A root the factory does not know is reported with a null clone rather than as absent: the SBT
    // says the root exists, so claiming otherwise would contradict a read we just made.
    const issuerClone = await reader.rootIssuer(addresses.factory, profileRootValue);
    const known = issuerClone && issuerClone !== ZERO_ADDRESS;
    const anchor = known
      ? await reader.anchorState(issuerClone, profileRootValue)
      : { issuedAt: 0n, revoked: false, valid: false };
    profile = {
      root: profileRootValue,
      issuerClone: known ? issuerClone : null,
      issuedAt: anchor.issuedAt,
      revoked: anchor.revoked,
      valid: anchor.valid,
    };
  }

  const ranges = chunkRanges(fromBlock, toBlock, chunk);
  const coverage: DiscoveryCoverage = {
    latestBlock,
    fromBlock,
    toBlock,
    reachedBlock: null,
    chunksTotal: ranges.length,
    chunksDone: 0,
    failures: [],
    complete: false,
    cancelled: false,
  };
  const events: DiscoveredEvent[] = [];
  // Newest first: scanning from the head means the operator sees the most recent — and for a
  // returning pet, the most relevant — activity while the older end is still in flight.
  let scannedTo = toBlock;

  for (const r of [...ranges].reverse()) {
    if (args.signal?.aborted) {
      coverage.cancelled = true;
      break;
    }
    try {
      const found = await reader.tagLogs({
        sbt: addresses.sbt,
        verificationRegistry: addresses.verificationRegistry,
        dogTagId,
        fromBlock: r.fromBlock,
        toBlock: r.toBlock,
      });
      events.push(...found);
    } catch (e) {
      // Recorded, never swallowed: this range is now UNKNOWN, and the panel must say so rather than
      // let a partial scan read as a complete one that found nothing.
      coverage.failures.push({
        fromBlock: r.fromBlock,
        toBlock: r.toBlock,
        message: (e as Error).message,
      });
    }
    coverage.chunksDone += 1;
    scannedTo = r.fromBlock;
    coverage.reachedBlock = r.fromBlock;
    args.onProgress?.({
      chunksDone: coverage.chunksDone,
      chunksTotal: coverage.chunksTotal,
      scannedTo,
      fromBlock,
      toBlock,
      found: events.length,
      failures: coverage.failures.length,
    });
  }

  coverage.complete =
    !coverage.cancelled &&
    coverage.chunksDone === coverage.chunksTotal &&
    coverage.failures.length === 0;

  // Newest first, and deterministically so: two events in one block are ordered by kind then tx hash
  // so a re-scan presents the same list rather than shuffling rows under the operator.
  events.sort(
    (a, b) =>
      Number(b.blockNumber - a.blockNumber) ||
      a.kind.localeCompare(b.kind) ||
      a.txHash.localeCompare(b.txHash),
  );

  return {
    dogTagId,
    status,
    statusLabel: tagStatusLabel(status),
    mintedBy,
    profile,
    events,
    coverage,
  };
}

/** Split `[from, to]` into inclusive chunks of at most `size` blocks. */
export function chunkRanges(
  from: bigint,
  to: bigint,
  size: bigint,
): Array<{ fromBlock: bigint; toBlock: bigint }> {
  const out: Array<{ fromBlock: bigint; toBlock: bigint }> = [];
  if (to < from) return out;
  for (let start = from; start <= to; start += size) {
    const end = start + size - 1n;
    out.push({ fromBlock: start, toBlock: end > to ? to : end });
  }
  return out;
}

/**
 * Whether an address is one of THIS shop's own signers, compared case-insensitively.
 *
 * This is what turns "a verification happened" into the finding that matters — a verification by a
 * relayer that is not us is evidence of credentials this shop has never seen, and therefore the cue
 * to ask the owner to share them. With no signers known, everything is reported as not-ours, which is
 * the safe direction: it overstates what is unfamiliar rather than claiming a stranger's record as
 * this shop's own work.
 */
export function isOwnSigner(addr: string | null | undefined, ownSigners: string[]): boolean {
  if (!addr) return false;
  const a = addr.trim().toLowerCase();
  return ownSigners.some((s) => s.trim().toLowerCase() === a);
}
