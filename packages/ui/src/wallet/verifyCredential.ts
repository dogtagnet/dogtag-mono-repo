// Submodule imports rather than the `@dogtag/standard` barrel — see the note in
// `../chain/tagDiscovery.ts`. The barrel drags in `consent.ts` -> `circomlibjs`, which references the
// Node `Buffer` global at module init and so throws in a browser; verification needs none of it.
import { checkIntegrity } from "@dogtag/standard/verify";
import { toHex32 } from "@dogtag/standard/field";
import type { WrappedDoc } from "@dogtag/standard/types";
import type { VerifyCredentialResp } from "../api/types";
import {
  DEPLOYED_ADDRESSES,
  isRootRevoked,
  isRootValid,
  issuedAtOf,
  isWhitelistedFor,
} from "./contracts";

/**
 * Permissionless, direct-to-RPC credential verification for the browser (mirrors the mobile apps).
 *
 * DogTag verification is fundamentally on-chain and permissionless: integrity is a pure offline
 * Merkle recompute (`@dogtag/standard` `checkIntegrity`), and issuance/revocation are plain `view`
 * reads on the per-record-type `DogTagIssuer` clone. The vet-api `POST /verify/credential` endpoint
 * is only an operator-gated *convenience relay* over those same reads - it adds no verification
 * authority. This module lets the web verify panel perform the identical check itself (viem
 * `eth_call` against the public ROAX RPC), so web verification no longer depends on that endpoint.
 *
 * The classification is a byte-for-byte port of the server handler (`stacks/vet/api/src/routes.rs`
 * `verify_credential`): same reads on the *claimed* root (`signature.merkleRoot`), same status enum,
 * same verdict formula. The core on-chain read (`DogTagIssuer.isValid`) is exactly what Android/iOS
 * `RoaxRpc.isValid` calls; `issuedAt`/`isRevoked` decompose it so the panel can distinguish
 * `not_issued` from `revoked`. Unlike mobile, an RPC failure fails closed (the promise rejects) so a
 * broken read is never silently treated as valid.
 */

/**
 * The four `view` reads verification needs, factored out so the orchestration is hermetically
 * testable with an injected fake. The default implementation ({@link roaxIssuerChainReader}) is
 * viem `eth_call` against the public ROAX RPC - no auth, no gas, no signer.
 */
export interface IssuerChainReader {
  /** DogTagIssuer.issuedAt(root) - anchoring unix timestamp, 0n when never issued. */
  issuedAt(issuerAddr: string, root: string): Promise<bigint>;
  /** DogTagIssuer.isValid(root) = issuedAt != 0 && !isRevoked. */
  isValid(issuerAddr: string, root: string): Promise<boolean>;
  /** DogTagIssuer.isRevoked(root) = revokedAt != 0. */
  isRevoked(issuerAddr: string, root: string): Promise<boolean>;
  /** IssuerRegistry.isWhitelistedFor(keccak256(recordType), signer). */
  isWhitelistedFor(registryAddr: string, recordType: string, signer: string): Promise<boolean>;
}

/** Default reader: viem `eth_call`s against the public ROAX RPC (chainId 135). */
export function roaxIssuerChainReader(rpcUrl?: string): IssuerChainReader {
  return {
    issuedAt: (issuerAddr, root) => issuedAtOf({ issuerAddr, root, rpcUrl }),
    isValid: (issuerAddr, root) => isRootValid({ issuerAddr, root, rpcUrl }),
    isRevoked: (issuerAddr, root) => isRootRevoked({ issuerAddr, root, rpcUrl }),
    isWhitelistedFor: (registryAddr, recordType, address) =>
      isWhitelistedFor({ registryAddr, recordType, address, rpcUrl }),
  };
}

export interface VerifyCredentialOnchainArgs {
  /** WrappedDoc JSON as produced by a DogTag issuer (already parsed). */
  wrappedDoc: Record<string, unknown>;
  /** Optional issuer signer address for the IssuerRegistry whitelist pillar. */
  signerAddr?: string;
  /** Optional DogTagIssuer clone override; defaults to `wrappedDoc.issuer.documentStore`. */
  issuerAddr?: string;
  /** IssuerRegistry address for the whitelist read; defaults to the deployed ROAX registry. */
  registryAddr?: string;
  /** Public RPC URL override; defaults to the ROAX devrpc configured in the chain definition. */
  rpcUrl?: string;
  /** Injected chain reader (tests); defaults to {@link roaxIssuerChainReader}. */
  reader?: IssuerChainReader;
  /** `checkedAt` unix-seconds override (tests); defaults to now. */
  now?: number;
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function stringField(v: unknown): string {
  return typeof v === "string" ? v : "";
}

/**
 * Verify a credential by reading the ROAX chain directly. Returns the same {@link VerifyCredentialResp}
 * shape the operator-gated endpoint returns, so the result renderer is unchanged. Throws only on a
 * structurally malformed document or an RPC read failure (the panel surfaces those as a toast).
 */
export async function verifyCredentialOnchain(
  args: VerifyCredentialOnchainArgs,
): Promise<VerifyCredentialResp> {
  const doc = args.wrappedDoc;
  const issuerMeta = doc.issuer;
  const signature = doc.signature;
  if (!isObject(issuerMeta) || !isObject(signature)) {
    throw new Error("Wrapped document is missing its issuer/signature fields.");
  }
  const recordType = stringField(issuerMeta.recordType);
  const documentStore = stringField(issuerMeta.documentStore);
  const claimedRoot = stringField(signature.merkleRoot);
  const issuerAddr = args.issuerAddr?.trim() || documentStore;
  if (!issuerAddr || !claimedRoot) {
    throw new Error(
      "Wrapped document is missing issuer.documentStore or signature.merkleRoot.",
    );
  }

  // Integrity pillar - pure offline recompute. A structurally-valid doc whose Merkle math does not
  // reconcile is an integrity failure (rendered), not a hard error.
  let integrityValid: boolean;
  let recomputedRoot: string;
  try {
    const { state, root } = checkIntegrity(doc as unknown as WrappedDoc);
    integrityValid = state === "VALID";
    recomputedRoot = toHex32(root);
  } catch {
    integrityValid = false;
    recomputedRoot = toHex32(0n);
  }

  const reader = args.reader ?? roaxIssuerChainReader(args.rpcUrl);
  const registryAddr = args.registryAddr?.trim() || DEPLOYED_ADDRESSES.IssuerRegistry;
  const signer = args.signerAddr?.trim();

  // On-chain pillars - all read the CLAIMED root (signature.merkleRoot), matching the server handler
  // and mobile; the recomputed root only ever populates the display field above.
  const [issuedAt, onchainValid, revoked, issuerWhitelisted] = await Promise.all([
    reader.issuedAt(issuerAddr, claimedRoot),
    reader.isValid(issuerAddr, claimedRoot),
    reader.isRevoked(issuerAddr, claimedRoot),
    signer ? reader.isWhitelistedFor(registryAddr, recordType, signer) : Promise.resolve(null),
  ]);
  const issued = issuedAt !== 0n;

  const status: VerifyCredentialResp["status"] = !integrityValid
    ? "integrity_failed"
    : !issued
      ? "not_issued"
      : revoked
        ? "revoked"
        : onchainValid
          ? "valid"
          : "invalid";
  const verdict = integrityValid && onchainValid && (issuerWhitelisted ?? true);

  return {
    verdict,
    status,
    recordType,
    root: claimedRoot,
    recomputedRoot,
    issuerAddr,
    signerAddr: signer || null,
    issuedAt: issuedAt.toString(),
    checkedAt: args.now ?? Math.floor(Date.now() / 1000),
    fragments: {
      integrity: integrityValid,
      onchain: onchainValid,
      issued,
      revoked,
      issuerWhitelisted,
    },
  };
}
