/**
 * Content addressing (registry-plan S-17).
 *
 * The address IS the hash of the bytes, so verification is RECOMPUTATION and nothing else. Where
 * the bytes arrived from is not evidence about them: a mirror that serves them, a CDN that caches
 * them and a provider that hosts them all have exactly the same standing, which is none.
 *
 * This module is the security boundary. `stacks/indexer/api/src/mirror.rs` checks on ingest and
 * again on read, but that is DEFENCE IN DEPTH — a client that skips its own recomputation because
 * "the mirror already checked" has turned content addressing back into trusting where the bytes
 * came from, which is the one thing content addressing exists to remove.
 *
 * Pure: no fetch, no DOM, no viem. The digest function is injected so the rule is testable without
 * a network and so a caller cannot quietly swap in a different algorithm than the one the anchor
 * declares.
 */

/** Lowercase `0x` + 64 hex. The same shape `ProfileAnchor.digest` holds on chain. */
export type ContentAddress = `0x${string}`;

/**
 * Multicodec code for keccak-256, matching `MULTIHASH_KECCAK_256` in
 * `packages/ui/src/provider/directoryPlan.ts` and `mirror.rs`, and what `ProfileAnchor.hashAlgorithm`
 * carries for everything this codebase publishes.
 */
export const MULTIHASH_KECCAK_256 = 0x1b;

/**
 * What a verification established.
 *
 * `mismatch` and `unsupportedAlgorithm` are BOTH failures to verify and both must render the same
 * way — nothing — but they are different accusations: one says these bytes are not the published
 * ones, the other says we do not know how to check. Collapsing them would tell a provider their
 * content was altered when the truth is that we could not compute the hash the anchor names.
 */
export type ContentVerification =
  | { readonly state: "verified"; readonly address: ContentAddress }
  | {
      readonly state: "mismatch";
      readonly requested: ContentAddress;
      readonly computed: ContentAddress;
    }
  | { readonly state: "unsupportedAlgorithm"; readonly hashAlgorithm: number };

/** Injected digest over raw bytes. The live surface passes viem's `keccak256`. */
export type BytesDigestFn = (bytes: Uint8Array) => string;

/** True when `value` is a well-formed content address, in either case. */
export function isContentAddress(value: string | null | undefined): value is ContentAddress {
  return typeof value === "string" && /^0x[0-9a-fA-F]{64}$/.test(value);
}

/** The all-zero word, which `ProfileAnchor.digest` uses for "nothing is anchored". */
export const ZERO_ADDRESS: ContentAddress = `0x${"0".repeat(64)}`;

/** True when this address names something. The zero word is an absence, never a content address. */
export function namesContent(value: string | null | undefined): value is ContentAddress {
  return isContentAddress(value) && value.toLowerCase() !== ZERO_ADDRESS;
}

function normalize(address: string): ContentAddress {
  return address.toLowerCase() as ContentAddress;
}

/**
 * Recompute the digest over `bytes` and compare it to the address they were requested under.
 *
 * `hashAlgorithm` is READ, never assumed. An anchor declaring an algorithm this build cannot compute
 * is `unsupportedAlgorithm` — recomputing with keccak anyway would report a genuine blob as altered,
 * which is the failure `directoryPlan.ts`'s own comment already names for the publish side.
 *
 * Case-insensitive on the requested address: a caller formatting a bytes32 as uppercase hex is an
 * ordinary wire difference, the same allowance `validate` makes for `versionId` and
 * `verificationRegistry` in `discovery.rs`.
 */
export function verifyContentAddress(
  bytes: Uint8Array,
  requestedAddress: string,
  hashAlgorithm: number,
  digest: BytesDigestFn,
): ContentVerification {
  if (hashAlgorithm !== MULTIHASH_KECCAK_256) {
    return { state: "unsupportedAlgorithm", hashAlgorithm };
  }
  if (!isContentAddress(requestedAddress)) {
    // A malformed address cannot be matched by anything, so nothing can verify against it. Reported
    // as a mismatch rather than a third state: the remedy is the same, and the computed value is
    // still worth carrying so a caller can see what the bytes actually were.
    return {
      state: "mismatch",
      requested: (requestedAddress || "0x") as ContentAddress,
      computed: normalize(digest(bytes)),
    };
  }
  const requested = normalize(requestedAddress);
  const computed = normalize(digest(bytes));
  return computed === requested
    ? { state: "verified", address: requested }
    : { state: "mismatch", requested, computed };
}
