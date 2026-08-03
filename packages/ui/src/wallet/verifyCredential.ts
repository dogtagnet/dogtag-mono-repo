// Submodule imports rather than the `@dogtag/standard` barrel — see the note in
// `../chain/tagDiscovery.ts`. The barrel drags in `consent.ts` -> `circomlibjs`, which references the
// Node `Buffer` global at module init and so throws in a browser; verification needs none of it.
import { checkIntegrity } from "@dogtag/standard/verify";
import { toHex32 } from "@dogtag/standard/field";
import type { WrappedDoc } from "@dogtag/standard/types";
import type { VerifyCredentialResp } from "../api/types";
import {
  DEPLOYED_ADDRESSES,
  grantInForceAt,
  isConfiguredAddress,
  isRootRevoked,
  isRootValid,
  issuedAtOf,
  issuedByOf,
  issuerRegistryOf,
  recordTypeKey,
  recordTypeOf,
  rootIssuedAtLog,
  rootIssuerOf,
  UNPOSITIONED_LOG,
  whitelistGrantHistory,
  ZERO_ADDRESS,
  ZERO_BYTES32,
  type LogPoint,
  type UnpositionedLog,
  type WhitelistGrantEvent,
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
 * The `view` reads verification needs, factored out so the orchestration is hermetically testable
 * with an injected fake. The default implementation ({@link roaxIssuerChainReader}) is viem
 * `eth_call` against the public ROAX RPC - no auth, no gas, no signer.
 */
export interface IssuerChainReader {
  /**
   * DogTagIssuerFactory.rootIssuer(root) - the clone that actually issued this root, zero address
   * when none did. The anchor every other read below is made against.
   */
  rootIssuer(factoryAddr: string, root: string): Promise<string>;
  /** DogTagIssuer.recordType() - the clone's own immutable record type key, zero word when unset. */
  recordType(issuerAddr: string): Promise<string>;
  /** DogTagIssuer.issuedAt(root) - anchoring unix timestamp, 0n when never issued. */
  issuedAt(issuerAddr: string, root: string): Promise<bigint>;
  /** DogTagIssuer.isValid(root) = issuedAt != 0 && !isRevoked. */
  isValid(issuerAddr: string, root: string): Promise<boolean>;
  /** DogTagIssuer.isRevoked(root) = revokedAt != 0. */
  isRevoked(issuerAddr: string, root: string): Promise<boolean>;
  /** DogTagIssuer.issuedBy(root) - the H-1 originator, zero address when never issued here. */
  issuedBy(issuerAddr: string, root: string): Promise<string>;
  /**
   * DogTagIssuer.registry() - the ONE registry whose `_wl` mapping `onlyWhitelisted` consults, and
   * therefore the only authority whose grant log answers for this contract's issuances. Written once
   * in `initialize` from the factory's own `immutable registry`, with no setter, so for a
   * factory-resolved clone it is as anchored as the clone itself.
   */
  issuerRegistry(issuerAddr: string): Promise<string>;
  /**
   * DogTagIssuer.RootIssued(root) - where this root was anchored, as a `(blockNumber, logIndex)`
   * point, or `null` when this contract emitted no such event. `issuedAt` is a unix TIMESTAMP and
   * cannot be compared against a log's height without a timestamp->block search.
   *
   * {@link UNPOSITIONED_LOG} when the node returned an anchoring log it gave no position - a third
   * outcome, never folded into either neighbour: `null` says the chain emitted nothing, while this
   * says we could not read where it emitted.
   */
  rootIssuedAt(
    issuerAddr: string,
    root: string,
  ): Promise<LogPoint | null | UnpositionedLog>;
  /**
   * `IssuanceCapabilitySet(service, signer, allowed)` - the full grant history for one pair, oldest
   * first. An empty array is a real answer ("no grant was ever recorded"); a read that FAILS rejects,
   * so the caller keeps that apart from "the log could not be reached"; and {@link UNPOSITIONED_LOG}
   * is the third, when some grant carried no position and so cannot be sequenced against the
   * anchoring point.
   */
  grantHistory(
    registryAddr: string,
    service: string,
    signer: string,
  ): Promise<WhitelistGrantEvent[] | UnpositionedLog>;
}

/**
 * Default reader: viem `eth_call`s against the public ROAX RPC (chainId 135).
 *
 * `blockNumber` pins every read to one height, so a batch of them is a consistent SNAPSHOT and the
 * resulting verdict is reproducible later - the posture the government verify route takes with its
 * `at_block`. Omitting it reads `latest`, which is what every caller did before the parameter existed.
 *
 * A caller that could not read the head MUST omit it and report its answer as unanchored. Passing a
 * head number that these reads were not actually pinned to would claim a snapshot that never happened.
 */
export function roaxIssuerChainReader(
  rpcUrl?: string,
  blockNumber?: bigint,
  defaultRpcUrl?: string,
): IssuerChainReader {
  return {
    rootIssuer: (factoryAddr, root) =>
      rootIssuerOf({ factoryAddr, root, rpcUrl, defaultRpcUrl, blockNumber }),
    recordType: (issuerAddr) =>
      recordTypeOf({ issuerAddr, rpcUrl, defaultRpcUrl, blockNumber }),
    issuedAt: (issuerAddr, root) =>
      issuedAtOf({ issuerAddr, root, rpcUrl, defaultRpcUrl, blockNumber }),
    isValid: (issuerAddr, root) =>
      isRootValid({ issuerAddr, root, rpcUrl, defaultRpcUrl, blockNumber }),
    isRevoked: (issuerAddr, root) =>
      isRootRevoked({ issuerAddr, root, rpcUrl, defaultRpcUrl, blockNumber }),
    issuedBy: (issuerAddr, root) =>
      issuedByOf({ issuerAddr, root, rpcUrl, defaultRpcUrl, blockNumber }),
    issuerRegistry: (issuerAddr) =>
      issuerRegistryOf({ issuerAddr, rpcUrl, defaultRpcUrl, blockNumber }),
    // The log reads share the report's own upper bound, so the whole answer is one snapshot: a grant
    // landing mid-verification cannot change a verdict printed under an earlier block.
    rootIssuedAt: (issuerAddr, root) =>
      rootIssuedAtLog({ issuerAddr, root, rpcUrl, defaultRpcUrl, toBlock: blockNumber }),
    grantHistory: (registryAddr, service, signer) =>
      whitelistGrantHistory({
        registryAddr,
        service,
        signer,
        rpcUrl,
        defaultRpcUrl,
        toBlock: blockNumber,
      }),
  };
}

export interface VerifyCredentialOnchainArgs {
  /** WrappedDoc JSON as produced by a DogTag issuer (already parsed). */
  wrappedDoc: Record<string, unknown>;
  /**
   * OPTIONAL *expected* issuer signer. The whitelist pillar no longer depends on this: the signer is
   * resolved from the chain (`issuedBy(root)`). Supplying it adds a strictly stronger assertion - the
   * pillar fails if the on-chain originator is not this address.
   */
  signerAddr?: string;
  /**
   * OPTIONAL *expected* DogTagIssuer clone. It can only TIGHTEN: every read is made against the clone
   * the FACTORY names for this root, so this asserts an expectation rather than selecting a contract.
   * Letting a caller pick the contract that answers for a credential would reopen the very forgery
   * this pillar exists to close, just through a different field.
   */
  issuerAddr?: string;
  /**
   * DEPRECATED for this function, and deliberately IGNORED: the issuer-whitelist pillar reads the
   * governing registry off the resolved clone's own `registry()`, because `IssuerRegistry._wl` and
   * its events are per-CONTRACT and only that instance's log can answer for this contract's
   * issuances. Honouring an override here would let a mis-paired client find no grant and print a
   * definite refusal of a genuine credential.
   *
   * Kept on the type because callers still pass it and other surfaces (the bench's
   * `registry-governs-issuer` row) legitimately compare it against the governing one to diagnose a
   * mis-paired configuration.
   */
  registryAddr?: string;
  /** DogTagIssuerFactory address used to resolve the issuing clone; defaults to the ROAX factory. */
  factoryAddr?: string;
  /** Public RPC URL override; defaults to the ROAX devrpc configured in the chain definition. */
  rpcUrl?: string;
  /** App-bundled endpoint used only after its own chain guard when the preferred endpoint fails. */
  defaultRpcUrl?: string;
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
  const documentStore = stringField(issuerMeta.documentStore).trim();
  const claimedRoot = stringField(signature.merkleRoot);
  if (!documentStore || !claimedRoot) {
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

  const reader =
    args.reader ?? roaxIssuerChainReader(args.rpcUrl, undefined, args.defaultRpcUrl);
  // The factory ALWAYS comes from this client's own configuration, never from the document. If the
  // document named it, an attacker would supply both sides of the question and the pillar would be
  // theatre. The registry is then derived from the clone the factory resolved, so it rests on the
  // same anchor - see the pillar below, and `args.registryAddr` for why an override is ignored.
  const factoryAddr = args.factoryAddr?.trim() || DEPLOYED_ADDRESSES.DogTagIssuerFactory;
  // NO FACTORY CONFIGURED IS ITS OWN REFUSAL, named. The pillar below is mandatory and anchors on
  // this address, so without it there is nothing to ask - and an empty `to` is not a contract that
  // answered. Left to fall through, viem rejects on a malformed address and the panel renders a
  // cryptic transport error for what is our own configuration gap. This path is deliberately
  // stricter than vet-api's silent `unavailableNoFactoryConfigured` degrade: the web verifier already
  // fails closed on an unreadable read, so it may not pass on an unmade one either.
  if (!isConfiguredAddress(factoryAddr)) {
    throw new Error(
      "no DogTagIssuerFactory configured: set VITE_DOGTAG_ISSUER_FACTORY_ADDR (from " +
        "contracts/deployments/roax.json). The issuer-whitelist pillar anchors on it, so this " +
        "credential cannot be verified without it.",
    );
  }
  const expectedSigner = args.signerAddr?.trim();

  // ANCHOR THE ISSUING CLONE - the read every on-chain pillar below is made against.
  //
  // The `issuer` block is NOT covered by the Merkle root, so `name`, `domain` and - the sharp one -
  // `documentStore` are chosen by whoever built the document. Asking `documentStore` whether the
  // credential is valid, and who issued it, is asking the suspect for their own references: deploy a
  // contract that answers `isValid = true` and names a genuinely whitelisted signer, and every pillar
  // passes on a wholly forged credential.
  //
  // So the clone is resolved from THIS client's `DogTagIssuerFactory`, whose `rootIssuer[R]` index is
  // written only from inside a clone's `issue()` (`require(isClone[msg.sender])`) and is strictly
  // write-once. A contract the factory never deployed can never appear there, and a genuine root's
  // issuer can never be overwritten.
  const rootIssuer = await reader.rootIssuer(factoryAddr, claimedRoot);
  const resolvedClone =
    rootIssuer && rootIssuer.toLowerCase() !== ZERO_ADDRESS ? rootIssuer : null;
  // `issuerAddr` is an EXPECTED-clone assertion that can only TIGHTEN, exactly like `signerAddr`. It
  // must never SELECT which contract answers, or the whole forgery just moves to a second field:
  // point it at an obliging contract and the factory anchor is bypassed without touching
  // `documentStore`. Both claims are compared against the factory's answer; neither replaces it.
  const expectedClone = args.issuerAddr?.trim();
  const cloneClaimsAgree =
    resolvedClone === null ||
    (resolvedClone.toLowerCase() === documentStore.toLowerCase() &&
      (!expectedClone || resolvedClone.toLowerCase() === expectedClone.toLowerCase()));

  // On-chain pillars - all read the CLAIMED root (signature.merkleRoot) on the RESOLVED clone; the
  // recomputed root only ever populates the display field above. With no clone there is nothing to
  // ask: no factory clone ever issued this root, so it is not valid anywhere.
  const [issuedAt, onchainValid, revoked] = resolvedClone
    ? await Promise.all([
        reader.issuedAt(resolvedClone, claimedRoot),
        reader.isValid(resolvedClone, claimedRoot),
        reader.isRevoked(resolvedClone, claimedRoot),
      ])
    : [0n, false, false];
  const issued = issuedAt !== 0n;

  // ISSUER-WHITELIST pillar, self-resolving and MANDATORY.
  //
  // It asks whether the on-chain originator held the capability AT THE MOMENT this root was anchored,
  // NOT whether it holds it now. Delisting is forward-only - `DogTagIssuer.sol:82` states the rule in
  // the contract's own source and `adminRevoke` is the retroactive lever - so a current-state
  // `isWhitelistedFor` read refuses every credential a rotated, retired or lapsed signer ever issued,
  // fleet-wide, while the protocol says each one is genuine. The answer is reconstructed from the
  // governing registry's own `Whitelisted`/`Delisted` logs, which anyone with an RPC can reproduce.
  //
  // Tri-state, and only a definite `true` may contribute to a pass:
  //   true  - resolved, and that signer held the capability for the record type the CHAIN says this
  //           root has, at the point it was anchored
  //   false - resolved, and it did not (or the envelope misrepresents the issuing clone / record
  //           type, or the caller's expected signer differs): a real authenticity failure
  //   null  - unresolvable (no factory clone ever issued this root, the clone names no issuer or no
  //           authority, the anchoring event could not be located, or a log came back with no
  //           position and so could not be ordered): INDETERMINATE, and an
  //           unanswered check is never a passed check
  let issuerWhitelisted: boolean | null = null;
  let resolvedSigner: string | null = null;
  if (resolvedClone && !cloneClaimsAgree) {
    // A claim points somewhere other than the contract that actually issued the root. Refused BEFORE
    // the registry is consulted - asking the whitelist about a document that already lied about its
    // issuer would only launder the answer.
    issuerWhitelisted = false;
  } else if (resolvedClone) {
    // The record type comes from the clone itself, never from the envelope: a signer authorized for
    // one record type must not be able to carry a credential relabelled as another. A clone reporting
    // no record type leaves the pillar indeterminate rather than falling back to the claim.
    const chainRecordTypeKey = await reader.recordType(resolvedClone);
    const hasChainRecordType =
      !!chainRecordTypeKey && chainRecordTypeKey.toLowerCase() !== ZERO_BYTES32;
    if (!hasChainRecordType) {
      issuerWhitelisted = null;
    } else if (
      chainRecordTypeKey.toLowerCase() !== recordTypeKey(recordType).toLowerCase()
    ) {
      issuerWhitelisted = false;
    } else {
      const originator = await reader.issuedBy(resolvedClone, claimedRoot);
      resolvedSigner =
        originator && originator.toLowerCase() !== ZERO_ADDRESS ? originator : null;
      if (resolvedSigner) {
        // WHICH authority answers, off the clone rather than off this client's configuration.
        // `IssuerRegistry._wl` and its events are per-CONTRACT, so a grant history read from a
        // different instance is a confident answer about a different mapping - and a client merely
        // pointed at the wrong registry would find no grant and refuse a genuine credential, which is
        // our own misconfiguration rendered as an accusation. This opens no trust surface: `registry`
        // is written once from the factory's own immutable, on a clone THIS client's factory resolved.
        const governing = await reader.issuerRegistry(resolvedClone);
        const anchoredAt =
          governing && governing.toLowerCase() !== ZERO_ADDRESS
            ? await reader.rootIssuedAt(resolvedClone, claimedRoot)
            : null;
        if (anchoredAt && anchoredAt !== UNPOSITIONED_LOG) {
          const history = await reader.grantHistory(
            governing,
            resolvedClone,
            resolvedSigner,
          );
          // A grant the node gave no position cannot be sequenced against the anchoring point, so the
          // fold cannot be run at all - and running it anyway, in either of the two available ways,
          // is a wrong answer rather than a rough one. Undetermined, exactly as the Rust, Kotlin and
          // Swift ports answer for the same log.
          if (history !== UNPOSITIONED_LOG) {
            // An EMPTY history is a DEFINITE refusal, and that is evidence about the credential
            // rather than about us: `issue()` is `onlyIssuanceCapable`, so an honest clone cannot
            // have anchored this root without the authority having granted this signer the
            // capability. A read that FAILED rejected above and never arrives here as an empty one.
            issuerWhitelisted = grantInForceAt(history, anchoredAt) === "authorized";
          }
        }
        // else: no authority to ask, no anchoring event to sequence against, or an anchoring log whose
        // position the node withheld. `issuerWhitelisted` stays `null` - never a pass, and never an
        // accusation drawn from a question we could not put.
        if (expectedSigner && expectedSigner.toLowerCase() !== resolvedSigner.toLowerCase()) {
          issuerWhitelisted = false;
        }
      }
    }
  }

  const status: VerifyCredentialResp["status"] = !integrityValid
    ? "integrity_failed"
    : !issued
      ? "not_issued"
      : revoked
        ? "revoked"
        : onchainValid
          ? "valid"
          : "invalid";
  const verdict = integrityValid && onchainValid && issuerWhitelisted === true;

  return {
    verdict,
    status,
    recordType,
    root: claimedRoot,
    recomputedRoot,
    // The clone the checks were actually made against, falling back to the envelope's claim only when
    // nothing resolved (so the operator sees what was asked about).
    issuerAddr: resolvedClone ?? documentStore,
    signerAddr: resolvedSigner,
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
