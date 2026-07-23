// Read-only ROAX access for the owner wallet. All reads are gasless (the owner never pays gas — the
// whole design keeps the holder gasless). DogTagIssuer.isValid(root) supplies the on-chain
// "issuance" pillar for a held credential; VerificationRegistryConsent's owner-blind `Verified`
// logs supply the owner's own consent history.
import { createPublicClient, decodeFunctionData, defineChain, http } from "viem";
import { toHex32 } from "@dogtag/standard";
import { ROAX_CHAIN_ID, ROAX_RPC_URL, VERIFICATION_REGISTRY_CONSENT_ADDR } from "./config";

export const roax = defineChain({
  id: ROAX_CHAIN_ID,
  name: "ROAX",
  nativeCurrency: { name: "Plasma", symbol: "PLASMA", decimals: 18 },
  rpcUrls: { default: { http: [ROAX_RPC_URL] } },
});

const publicClient = createPublicClient({ chain: roax, transport: http(ROAX_RPC_URL) });

const ISSUER_ABI = [
  {
    type: "function",
    name: "isValid",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "bool" }],
  },
] as const;

/** DogTagIssuer clone (== issuer.documentStore) isValid(root) — true once the root is anchored. */
export async function isRootAnchored(documentStore: string, merkleRoot: string): Promise<boolean> {
  return publicClient.readContract({
    address: documentStore as `0x${string}`,
    abi: ISSUER_ABI,
    functionName: "isValid",
    args: [merkleRoot as `0x${string}`],
  });
}

/** The owner-blind `Verified` event (`VerificationRegistryConsent.sol`): no owner/subject field -
 *  `dogTagId` is the opaque field-hashed tag id, so only a holder who already knows their own tag
 *  ids (from held credentials) can attribute a log to themselves. */
const VERIFIED_EVENT = {
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
} as const;

/** `recordVerificationZK(a, b, c, pub[7])` - decoded only to read back the public signals the
 *  Verified event does not carry (`pub[5]` = recordType). Frozen with the M3 consent VK. */
const RECORD_VERIFICATION_ZK_ABI = [
  {
    type: "function",
    name: "recordVerificationZK",
    stateMutability: "nonpayable",
    inputs: [
      { name: "a", type: "uint256[2]" },
      { name: "b", type: "uint256[2][2]" },
      { name: "c", type: "uint256[2]" },
      { name: "pub", type: "uint256[7]" },
    ],
    outputs: [],
  },
] as const;

/** `recordType`'s index in the frozen consent public-signal order
 *  `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]` - signals are read via named
 *  constants, never a literal index (repo doctrine, the E-1 transposition class). */
const PUB_RECORD_TYPE = 5;

/** One decoded owner-blind `Verified` log, plus its chain provenance. */
export interface VerifiedLog {
  dogTagId: bigint;
  relayer: `0x${string}`;
  purpose: `0x${string}`;
  nullifier: `0x${string}`;
  deadline: bigint;
  ts: bigint;
  txHash: `0x${string}`;
  blockNumber: bigint;
}

/**
 * Every `Verified` log for the given (already field-hashed) tag ids, oldest→newest as the node
 * returns them. One indexed-topic `eth_getLogs` over the registry's whole history - ROAX devnet
 * serves unbounded ranges, and an owner's own consent history is small by construction.
 */
export async function fetchVerifiedLogs(dogTagIds: bigint[]): Promise<VerifiedLog[]> {
  if (dogTagIds.length === 0) return [];
  const logs = await publicClient.getLogs({
    address: VERIFICATION_REGISTRY_CONSENT_ADDR,
    event: VERIFIED_EVENT,
    args: { dogTagId: dogTagIds },
    fromBlock: 0n,
    toBlock: "latest",
    strict: true,
  });
  return logs.map((l) => ({
    dogTagId: l.args.dogTagId,
    relayer: l.args.relayer,
    purpose: l.args.purpose,
    nullifier: l.args.nullifier,
    deadline: l.args.deadline,
    ts: l.args.ts,
    txHash: l.transactionHash,
    blockNumber: l.blockNumber,
  }));
}

/**
 * The proof's `recordType` public signal (`pub[5]`) as its 0x…32-byte hex, recovered from the
 * verifying transaction's calldata - the owner-blind event deliberately omits it. `null` when the
 * tx cannot be read, is not a plain `recordVerificationZK` call (e.g. a future registry wraps the
 * call differently), or carries a value outside the field; callers render that as "unavailable",
 * never as a failure of the whole history.
 */
export async function fetchConsentRecordType(txHash: `0x${string}`): Promise<string | null> {
  try {
    const tx = await publicClient.getTransaction({ hash: txHash });
    const { args } = decodeFunctionData({ abi: RECORD_VERIFICATION_ZK_ABI, data: tx.input });
    return toHex32(args[3][PUB_RECORD_TYPE]);
  } catch {
    return null;
  }
}
