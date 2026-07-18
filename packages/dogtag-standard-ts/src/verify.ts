// Three-pillar contextual verification (impl §11.3 — supersedes §1.7).
// Validity = integrity AND issuance AND identity (the 3 authenticity pillars). `ownership` is a
// CONTEXTUAL 4th fragment: gates only the owner's self-import; NOT_APPLICABLE for third parties.
import {buildMerkle} from "./merkle.js";
import {flattenData, leafFromPacked} from "./wrap.js";
import {fromHex32, toHex32, type Field} from "./field.js";
import type {FragmentState, Verdict, WrappedDoc} from "./types.js";

/** Network adapters are injected so the core SDK stays pure/offline (mobile + server share it). */
export interface RpcAdapter {
  /** DogTagIssuer.isValid(root) at >= `confirmations` blocks. Throw to signal a transient ERROR. */
  isValid(documentStore: string, merkleRoot: string, confirmations: number): Promise<boolean>;
  /** DogTagSBT.ownerOf(dogTagId). Throw to signal a transient ERROR. */
  ownerOf(dogTagId: string): Promise<string>;
  /**
   * DogTagIssuer.issuedBy(root) - the authoritative signer that issued `R` (== `clone.issuedBy[R]`),
   * used to validate the envelope's `protocol.issuerSigner` *claim* (§4.3). OPTIONAL: when absent
   * (unwired), the additive issuer-signer check is skipped and base validity governs. Live per-backend
   * eth-calls are the later verify-path hardening brick (M7 P5); the SDK enforces it whenever wired.
   */
  issuedBy?(documentStore: string, merkleRoot: string): Promise<string>;
}
export interface DnsAdapter {
  /** True iff a TXT record of `domain` binds `documentStore` on `chainId`. Throw for ERROR. */
  txtMatches(domain: string, documentStore: string, chainId: number): Promise<boolean>;
}
export interface RegistryAdapter {
  /** The admin-written central registry knows this (domain, documentStore) pair. */
  knows(domain: string, documentStore: string): Promise<boolean>;
}

export interface VerifyOpts {
  rpc: RpcAdapter;
  dns: DnsAdapter;
  registry: RegistryAdapter;
  mode: "self-import" | "third-party";
  userWalletAddress?: string;
  confirmations?: number;
}

/** Paths that must be present and are NON-obfuscatable (audit-05 V3/V6). */
const NON_OBFUSCATABLE = ["credentialSubject.dogTagId"];

const HEX32 = /^0x[0-9a-fA-F]{64}$/;

/**
 * Pure integrity pillar: rebuild the WHOLE tree (never trust processProof alone — C1) and
 * compare to targetHash, then resolve the proof to merkleRoot. Returns the recomputed root + state.
 */
export function checkIntegrity(doc: WrappedDoc): {state: FragmentState; root: Field} {
  for (const h of doc.privacy.obfuscated) {
    if (!HEX32.test(h)) return {state: "INVALID", root: 0n};
  }
  const dataFlat = flattenData(doc.data);
  const presentPaths = new Set(dataFlat.map(([kp]) => kp));
  for (const req of NON_OBFUSCATABLE) {
    if (!presentPaths.has(req)) return {state: "INVALID", root: 0n}; // required + non-obfuscatable
  }
  const liveLeaves: Field[] = dataFlat.map(([kp, packed]) => leafFromPacked(kp, packed));
  const obf = doc.privacy.obfuscated.map(fromHex32);
  // obfuscated entries must not overlap live-leaf hashes (D1)
  const liveSet = new Set(liveLeaves.map((x) => x.toString()));
  for (const o of obf) {
    if (liveSet.has(o.toString())) return {state: "INVALID", root: 0n};
  }
  const {root} = buildMerkle([...liveLeaves, ...obf]);
  const targetHash = fromHex32(doc.signature.targetHash);
  if (root !== targetHash) return {state: "INVALID", root};
  const merkleRoot = fromHex32(doc.signature.merkleRoot);
  // Single-document credentials only: `signature.proof` MUST be empty, so `targetHash` IS the
  // anchored root `R`. Doc→batch-root inclusion (a non-empty `proof`) never shipped, and the C1
  // invariant forbids trusting a permissive commutative fold in the trust path — so a non-empty
  // proof is rejected outright rather than folded (see merkle.processProof, DSDP plan §2.3).
  const ok = doc.signature.proof.length === 0 && merkleRoot === targetHash;
  return {state: ok ? "VALID" : "INVALID", root};
}

function dogTagIdOf(doc: WrappedDoc): string {
  const entry = flattenData(doc.data).find(([kp]) => kp === "credentialSubject.dogTagId");
  if (!entry) throw new Error("missing credentialSubject.dogTagId");
  // packed: salt:tag:value
  const parts = entry[1].split(":");
  return parts.slice(2).join(":");
}

/** Full contextual verify (impl §11.3). */
export async function verify(doc: WrappedDoc, opts: VerifyOpts): Promise<Verdict> {
  const confirmations = opts.confirmations ?? 5;
  const integrity = checkIntegrity(doc).state;

  let issuance: FragmentState;
  try {
    issuance = (await opts.rpc.isValid(doc.issuer.documentStore, doc.signature.merkleRoot, confirmations))
      ? "VALID"
      : "INVALID";
  } catch {
    issuance = "ERROR";
  }

  // M7 provenance (§4.2/§4.3): a stamped `protocol.issuerSigner` is only the envelope's CLAIM of who
  // issued; validate it against the authoritative on-chain `clone.issuedBy[R]`. A wrong/forged claim
  // fails closed (issuance -> INVALID). This can only make verification STRICTER - the on-chain
  // validity re-derivation above still targets `doc.issuer.documentStore`, NEVER the untrusted block,
  // so a forged `protocol` block can neither reroute validation nor make an invalid record verify.
  // Skipped when the block is absent (pre-M7) or the adapter is unwired - base validity governs.
  // Mirror of the Rust `verify`; live per-backend `issuedBy` reads are the later hardening brick (P5).
  if (doc.protocol && issuance === "VALID" && opts.rpc.issuedBy) {
    try {
      const onchain = await opts.rpc.issuedBy(doc.issuer.documentStore, doc.signature.merkleRoot);
      issuance = onchain.toLowerCase() === doc.protocol.issuerSigner.toLowerCase() ? "VALID" : "INVALID";
    } catch {
      // transient/unwired read -> skip the additive check; base validity governs (mirror Rust `Err -> Valid`).
      issuance = "VALID";
    }
  }

  let identity: FragmentState;
  try {
    const txt = await opts.dns.txtMatches(doc.issuer.domain, doc.issuer.documentStore, 135);
    const known = await opts.registry.knows(doc.issuer.domain, doc.issuer.documentStore);
    identity = txt && known ? "VALID" : "INVALID";
  } catch {
    identity = "ERROR";
  }

  const credentialValid = integrity === "VALID" && issuance === "VALID" && identity === "VALID";

  let ownership: FragmentState;
  let valid: boolean;
  if (opts.mode === "self-import") {
    if (!opts.userWalletAddress) throw new Error("self-import requires userWalletAddress");
    try {
      const owner = await opts.rpc.ownerOf(dogTagIdOf(doc));
      ownership = owner.toLowerCase() === opts.userWalletAddress.toLowerCase() ? "VALID" : "INVALID";
    } catch {
      ownership = "ERROR";
    }
    valid = credentialValid && ownership === "VALID";
  } else {
    if (opts.userWalletAddress) {
      try {
        const owner = await opts.rpc.ownerOf(dogTagIdOf(doc));
        ownership = owner.toLowerCase() === opts.userWalletAddress.toLowerCase() ? "VALID" : "INVALID";
      } catch {
        ownership = "ERROR";
      }
    } else {
      ownership = "NOT_APPLICABLE";
    }
    valid = credentialValid; // ownership does NOT gate third-party validity
  }

  return {valid, fragments: {integrity, issuance, identity, ownership}};
}
