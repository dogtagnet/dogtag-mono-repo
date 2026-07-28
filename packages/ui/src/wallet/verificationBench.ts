// Submodule imports rather than the `@dogtag/standard` barrel - see the note at the top of
// `./verifyCredential.ts`. The barrel drags in `consent.ts` -> `circomlibjs`, which touches the Node
// `Buffer` global at module init and throws in a browser.
import { checkIntegrity } from "@dogtag/standard/verify";
import { toHex32 } from "@dogtag/standard/field";
import { flattenData, parsePacked } from "@dogtag/standard/wrap";
import type { WrappedDoc } from "@dogtag/standard/types";
import type { VerifyCredentialResp } from "../api/types";
import { DEPLOYED_ADDRESSES, ZERO_ADDRESS, issuerDomainClaimOf } from "./contracts";
import {
  roaxIssuerChainReader,
  verifyCredentialOnchain,
  type IssuerChainReader,
  type VerifyCredentialOnchainArgs,
} from "./verifyCredential";

/**
 * The admin verification bench: run the REAL verifier over a record and report, per check, whether it
 * passed, failed, or could not run - with the evidence each answer rests on.
 *
 * # The one property this module exists to preserve
 *
 * `runVerificationBench` does NOT re-implement verification. It calls
 * {@link verifyCredentialOnchain} - the same function the wallet and the credential verify panel call -
 * and copies its verdict verbatim. A bench that computed its own answer would agree with itself and
 * prove nothing; the whole point is to make the REAL path's reasoning legible.
 *
 * # How per-check evidence is obtained without touching the verifier
 *
 * The verifier reports fragments but not provenance: `issuerAddr` is `resolvedClone ?? documentStore`,
 * so its response alone cannot say WHETHER the factory resolved the clone or the document's own claim
 * was used as a fallback - and that distinction is the entire issuer pillar. Rather than change the
 * verifier or duplicate its logic, the bench wraps its `IssuerChainReader` seam in a transparent
 * {@link recordingReader} and OBSERVES the reads it makes: which contract was asked, what it answered,
 * and at which block. Every "evidence" line below is a recorded observation, never a recomputation.
 *
 * # THE INVARIANT
 *
 * NO ROW MAY ASSERT AN OUTCOME OR AN EVIDENCE LINE FROM A VALUE THE RECORDER NEVER OBSERVED.
 *
 * `verifyCredentialOnchain` short-circuits when the factory resolves no clone: it fills
 * `[issuedAt, onchainValid, revoked]` with `[0n, false, false]` having made NO chain access at all, and
 * sets `issuerAddr` to the document's own `documentStore`. Those defaults are ABSENCE OF EVIDENCE. Read
 * as evidence they produce the exact collapse this surface exists to prevent - a green "not revoked"
 * row citing a read never made, against an address the rest of the codebase deliberately refuses to
 * trust.
 *
 * Patching the rows that happen to be wrong today would fix the instance; the invariant makes the whole
 * class impossible. A bench that fabricates an evidence line is worse than a bench that is merely
 * wrong, because it hands the reader a citation for a read that never happened - and citations are the
 * entire product of this surface.
 *
 * It is enforced structurally rather than by review: a chain citation can only be built by
 * {@link readEvidence}, which takes the recorded {@link ChainRead} itself rather than an address and a
 * method name a caller assembled. With no read in the log there is nothing to pass, so the citation
 * cannot be written. {@link readBackedCheck} applies the same rule to outcomes: a row whose licensing
 * read is absent or failed reports `could-not-run` naming the read that never happened. Nothing is lost
 * by this - when no clone resolves, `issuer-descends-from-factory` already reports a definite FAIL,
 * which is the honest place for that signal.
 *
 * # Why "could not run" is a first-class outcome here
 *
 * `verifyCredentialOnchain` fails CLOSED: a read that throws rejects the whole promise, so there is no
 * verdict at all. That is correct for a verifier - a broken read must never read as valid - but it
 * leaves an operator unable to tell "this credential is bad" from "we could not ask". The bench keeps
 * the two apart: a check whose evidence never arrived reports {@link CheckOutcome} `could-not-run` with
 * the reason, and NEVER as a pass or a failure. Collapsing that third state in either direction is the
 * defect this surface exists to make impossible.
 */

/**
 * A check's outcome. Three states, never two.
 *
 * `could-not-run` must never be rendered as either neighbour. It is not a soft failure: a chain we
 * could not reach is not evidence about the credential, and an absent field is not a forgery.
 */
export type CheckOutcome = "pass" | "fail" | "could-not-run";

export type BenchCheckId =
  | "integrity"
  | "issuer-descends-from-factory"
  | "document-names-issuing-contract"
  | "issuer-whitelisted"
  | "anchored-on-chain"
  | "not-revoked"
  | "not-expired"
  | "issuer-domain-claim"
  | "issuer-domain-dns";

/**
 * Which checks the VERIFIER'S OWN verdict folds in, and which are reported beside it.
 *
 * `verifyCredentialOnchain` computes `integrity && onchain && issuerWhitelisted === true`. Expiry and
 * the issuer-domain checks feed none of that, so an expired-but-anchored credential legitimately
 * produces `verdict: true` with a red expiry row - the captain's own point that an expired credential
 * is on-chain-valid. Stating which rows gate is what stops that reading as a self-contradiction: a
 * surface that showed a green banner over a red row without saying why would be leaving the reader to
 * derive the reason, on the one page whose entire product is not making them do that.
 *
 * `anchored-on-chain` and `not-revoked` DO gate: they are the decomposition of the verifier's single
 * `onchain` term (`isValid == issuedAt != 0 && !isRevoked`), split apart so the row can say which half
 * objected.
 */
const GATES_VERDICT: Record<BenchCheckId, boolean> = {
  integrity: true,
  "issuer-descends-from-factory": true,
  "document-names-issuing-contract": true,
  "issuer-whitelisted": true,
  "anchored-on-chain": true,
  "not-revoked": true,
  // Reported BESIDE the verdict, never folded into it.
  "not-expired": false,
  "issuer-domain-claim": false,
  "issuer-domain-dns": false,
};

/** One thing that was read, and from where. Rendered verbatim so a reader can reproduce it by hand. */
export interface EvidenceLine {
  label: string;
  value: string;
  /** The contract + method the value came from, or `"offline"` for a pure local computation. */
  source: string;
}

export interface BenchCheck {
  id: BenchCheckId;
  /** The check as a plain-language question, not a jargon label. */
  question: string;
  outcome: CheckOutcome;
  /**
   * Whether the VERIFIER'S verdict folds this check in. A `false` row can be red while the verdict is
   * `true` - see {@link GATES_VERDICT} for why that is a real property and not a contradiction.
   */
  gatesVerdict: boolean;
  /** A one-line statement of what was observed. Always present, for every outcome. */
  finding: string;
  /**
   * Why the check could not run. Present if and ONLY if `outcome === "could-not-run"`, so a renderer
   * cannot accidentally show a reason beside a pass, or omit one beside a could-not-run.
   */
  couldNotRunReason?: string;
  evidence: EvidenceLine[];
}

/** One observed chain read. `failed` and "absent from this list entirely" are different facts. */
export interface ChainRead {
  method: keyof IssuerChainReader | "issuerDomainClaim";
  /** The contract the question was actually put to. */
  contract: string;
  args: string[];
  outcome: "ok" | "failed";
  /** The answer, when `outcome === "ok"`. */
  value?: string;
  /** The failure, when `outcome === "failed"`. */
  error?: string;
}

export interface BenchReport {
  /**
   * The REAL verifier's verdict, copied verbatim. `null` means it produced none - it fails closed on a
   * read failure or a malformed envelope, and an absent verdict must never be shown as `false`.
   */
  verdict: boolean | null;
  /** The verifier's full response, when it returned one. */
  response: VerifyCredentialResp | null;
  /** Why the verifier could not answer at all. `null` when it did. */
  verifierError: string | null;
  /**
   * The block every on-chain read was pinned to. `null` means the head could not be read, the reads
   * therefore used `latest`, and this report is NOT reproducible - said plainly rather than implied.
   */
  blockNumber: bigint | null;
  checks: BenchCheck[];
  /** Every read attempted, in order, as observed. The raw material behind `evidence`. */
  reads: ChainRead[];
}

/**
 * Wrap a reader so every call is recorded, then pass the wrapper to the real verifier.
 *
 * Transparent by construction: each method forwards its arguments unchanged and re-throws the original
 * error after recording it. It can only observe - it can neither soften a failure into a value nor
 * change which contract is asked, which is what makes the recorded log admissible as evidence about the
 * verifier's own behaviour rather than about the wrapper's.
 */
export function recordingReader(inner: IssuerChainReader): {
  reader: IssuerChainReader;
  reads: ChainRead[];
} {
  const reads: ChainRead[] = [];
  async function record<T>(
    method: ChainRead["method"],
    contract: string,
    args: string[],
    run: () => Promise<T>,
  ): Promise<T> {
    try {
      const value = await run();
      reads.push({ method, contract, args, outcome: "ok", value: String(value) });
      return value;
    } catch (e) {
      reads.push({
        method,
        contract,
        args,
        outcome: "failed",
        error: e instanceof Error ? e.message : String(e),
      });
      throw e;
    }
  }
  const reader: IssuerChainReader = {
    rootIssuer: (factory, root) =>
      record("rootIssuer", factory, [root], () => inner.rootIssuer(factory, root)),
    recordType: (addr) => record("recordType", addr, [], () => inner.recordType(addr)),
    issuedAt: (addr, root) => record("issuedAt", addr, [root], () => inner.issuedAt(addr, root)),
    isValid: (addr, root) => record("isValid", addr, [root], () => inner.isValid(addr, root)),
    isRevoked: (addr, root) => record("isRevoked", addr, [root], () => inner.isRevoked(addr, root)),
    issuedBy: (addr, root) => record("issuedBy", addr, [root], () => inner.issuedBy(addr, root)),
    isWhitelistedFor: (registry, key, signer) =>
      record("isWhitelistedFor", registry, [key, signer], () =>
        inner.isWhitelistedFor(registry, key, signer),
      ),
  };
  return { reader, reads };
}

/** The single on-chain domain read the bench makes on its own account. See {@link issuerDomainClaimOf}. */
export interface DomainClaimReader {
  claim(
    registryAddr: string,
    cloneAddr: string,
  ): Promise<{ domain: string; updatedAtBlock: bigint } | null>;
}

export interface BenchInput
  extends Pick<VerifyCredentialOnchainArgs, "registryAddr" | "factoryAddr" | "rpcUrl" | "reader"> {
  /** The record under test: a parsed WrappedDoc. */
  wrappedDoc: Record<string, unknown>;
  /**
   * `IssuerDomainRegistry` address. Deliberately has NO default, unlike the factory and the issuer
   * registry: the contract set is still under revision and this one may yet be folded elsewhere.
   * Unset means the domain check reports itself unavailable - never a pass, and never a failure.
   */
  domainRegistryAddr?: string;
  /** Injected domain reader (tests); defaults to a viem read pinned to `blockNumber`. */
  domainClaimReader?: DomainClaimReader;
  /**
   * The block to pin every on-chain read to. Omit when the head could not be read: the reads then use
   * `latest` and the report says it is unanchored, rather than naming a block it never saw.
   */
  blockNumber?: bigint;
  /** Today as an ISO `YYYY-MM-DD` UTC date, for the expiry check. Defaults to now. */
  today?: string;
  /** `checkedAt` override, forwarded to the verifier (tests). */
  now?: number;
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

/** Today's UTC date as `YYYY-MM-DD`. Mirrors the backend's `today_iso()`. */
export function todayIso(at: Date = new Date()): string {
  return at.toISOString().slice(0, 10);
}

/**
 * THE canonical three-tier `validUntil` chain, in precedence order. One list, three issuers:
 *
 *  1. `credentialSubject.validity.validUntil` - TRAVEL_CLEARANCE's nested CDC validity block.
 *  2. `credentialSubject.rabiesValidUntil`    - EU_HEALTH_CERT's flat Annex-IV leaf. It has no
 *     `validity` block at all, so a chain that omits this tier reports every EU health certificate as
 *     carrying no validity window - on the one surface built to expose expiry.
 *  3. `validUntil` - VACCINATION's schema field is DOTLESS, so it lands as a SIBLING of
 *     `credentialSubject` at the top level of `data`, never as a child of it.
 *
 * `credentialSubject.validUntil` is deliberately NOT in the list: no issuer emits it.
 *
 * The same chain is implemented in `stacks/owner/web/src/lib/receipt.ts` and in both mobile
 * `WrappedDoc.validUntil` ports, explicitly so the surfaces cannot drift about when a credential has
 * lapsed. Move all of them together or none.
 */
export const VALID_UNTIL_KEYPATHS = [
  "credentialSubject.validity.validUntil",
  "credentialSubject.rabiesValidUntil",
  "validUntil",
] as const;

export interface ValidityWindow {
  keyPath: string;
  /** The unpacked leaf value, exactly as the document carries it - blank and malformed included. */
  value: string;
  /**
   * Whether `value` can be compared. A leaf that is present but blank or not ISO-shaped is NOT a
   * lapsed window: `today > ""` is true for every date, so comparing it would accuse a credential
   * whose window was simply left empty at issuance.
   */
  usable: boolean;
}

/** `YYYY-MM-DD`, optionally with a time suffix - the shape a plain-string date compare is sound on. */
function isIsoDate(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}/.test(value);
}

/**
 * The credential's validity window, unpacked from the Merkle-covered `data`.
 *
 * Walks {@link VALID_UNTIL_KEYPATHS} and returns the first USABLE tier. Emptiness is tested on the
 * UNWRAPPED value, so a present-but-blank tier falls through to the next - matching the canonical
 * chain's own behaviour. When a tier is present but no tier is usable, the first present one is
 * returned with `usable: false`, so the check can say "the window is unreadable" rather than the
 * different and untrue "this document carries no window".
 *
 * The value is a PACKED `salt:tag:value` leaf, so it is unpacked with the SDK's own `parsePacked`
 * rather than by splitting on `:` here - the value itself may contain colons.
 */
export function validUntilOf(doc: Record<string, unknown>): ValidityWindow | null {
  const flat = flattenData(doc.data);
  let firstPresent: ValidityWindow | null = null;
  for (const keyPath of VALID_UNTIL_KEYPATHS) {
    const entry = flat.find(([kp]) => kp === keyPath);
    if (!entry) continue;
    let value: string;
    try {
      value = parsePacked(entry[1]).valueRest.trim();
    } catch {
      // A leaf too malformed to unpack carries no readable date, but the document DOES claim a window
      // here - so it is recorded as present-and-unusable rather than treated as absent.
      firstPresent ??= { keyPath, value: "", usable: false };
      continue;
    }
    if (value && isIsoDate(value)) return { keyPath, value, usable: true };
    firstPresent ??= { keyPath, value, usable: false };
  }
  return firstPresent;
}

/**
 * Pull the wrapped document out of whatever a share endpoint returned.
 *
 * A DogTag credential QR encodes a share URL, and the endpoints behind those URLs are not uniform:
 * some return the `WrappedDoc` directly, others wrap it as `wrappedDoc` (web) or `wrapped_doc` (the
 * Rust routes' serde name). Returning the body unchanged when neither key is present is what lets a
 * bare-document endpoint work; it is NOT a guess about validity - a body that is not a document simply
 * fails the checks below and says so.
 */
export function docFromShareResponse(body: unknown): Record<string, unknown> {
  if (!isObject(body)) return {} as Record<string, unknown>;
  for (const key of ["wrappedDoc", "wrapped_doc"]) {
    const inner = body[key];
    if (isObject(inner)) return inner;
  }
  return body;
}

/** The last read of `method`, or `undefined` when it was never attempted. */
function lastRead(reads: ChainRead[], method: ChainRead["method"]): ChainRead | undefined {
  for (let i = reads.length - 1; i >= 0; i--) {
    const r = reads[i];
    if (r && r.method === method) return r;
  }
  return undefined;
}

function isZeroAddr(v: string | undefined): boolean {
  return !!v && v.toLowerCase() === ZERO_ADDRESS;
}

/**
 * The ONLY way to cite a chain read as evidence, and the mechanism behind THE INVARIANT above.
 *
 * It takes the recorded {@link ChainRead} rather than an address and a method name the caller
 * assembled, so a citation for a read that was never made cannot be written: there is no read to pass.
 * The value and the contract both come off the observation, never off the verifier's response - which
 * reports `issuerAddr` as `resolvedClone ?? documentStore` and so cannot say which of the two answered.
 *
 * Every source it emits ends `on <contract>`; nothing else in this module produces that shape, which is
 * what lets a test assert generically that no cited contract is absent from the recorded reads.
 */
function readEvidence(label: string, read: ChainRead, call: string): EvidenceLine {
  return {
    label,
    value:
      read.outcome === "ok"
        ? (read.value ?? "")
        : `(unanswered - the read failed: ${read.error ?? "unknown error"})`,
    source: `${call} on ${read.contract}`,
  };
}

/**
 * Why a row backed by `method` could not assert anything, phrased from what the recorder actually saw.
 *
 * The three cases are kept apart because their remedies are: a read that FAILED is a chain we could not
 * reach, a read that was NEVER MADE against a resolved clone is a verifier that stopped earlier, and a
 * read never made because no clone resolved is the case where the document's own `documentStore` was
 * deliberately not asked in the factory's place.
 */
function unobservedReason(
  reads: ChainRead[],
  read: ChainRead | undefined,
  method: ChainRead["method"],
  verifierError: string | null,
): string {
  if (read?.outcome === "failed") {
    return `The chain read \`${method}\` against ${read.contract} failed: ${read.error ?? "unknown error"}`;
  }
  if (!read) {
    const anchor = lastRead(reads, "rootIssuer");
    if (anchor?.outcome === "ok" && (isZeroAddr(anchor.value) || !anchor.value)) {
      return `No factory-deployed contract was resolved for this root, so \`${method}\` was never asked of anything. The contract the document names was deliberately NOT asked in its place, so nothing is known either way.`;
    }
    return (
      failedReadReason(reads) ??
      (verifierError
        ? `The verifier stopped before \`${method}\` was read: ${verifierError}`
        : `No \`${method}\` read was made.`)
    );
  }
  return (
    failedReadReason(reads) ??
    (verifierError ? `The verifier stopped before this check: ${verifierError}` : "The check was not reached.")
  );
}

/**
 * A row licensed by ONE observed chain read: it may assert only when that read is in the log and
 * succeeded, and its evidence is built from the read itself. See THE INVARIANT at the top of the file.
 *
 * The verdict still comes from the verifier's own response - `decide` reads its fragments rather than
 * recomputing them - so this gates whether a claim may be made, not what the claim is.
 */
function readBackedCheck(args: {
  id: BenchCheckId;
  question: string;
  method: ChainRead["method"];
  call: string;
  label: string;
  reads: ChainRead[];
  response: VerifyCredentialResp | null;
  verifierError: string | null;
  /** The finding when the row cannot run - what was NOT established. */
  unestablished: string;
  decide(response: VerifyCredentialResp): { outcome: "pass" | "fail"; finding: string };
}): Omit<BenchCheck, "gatesVerdict"> {
  const read = lastRead(args.reads, args.method);
  const evidence = read ? [readEvidence(args.label, read, args.call)] : [];
  if (!read || read.outcome !== "ok" || !args.response) {
    return {
      id: args.id,
      question: args.question,
      outcome: "could-not-run",
      finding: args.unestablished,
      couldNotRunReason: unobservedReason(args.reads, read, args.method, args.verifierError),
      evidence,
    };
  }
  const decided = args.decide(args.response);
  return {
    id: args.id,
    question: args.question,
    outcome: decided.outcome,
    finding: decided.finding,
    evidence,
  };
}

/**
 * Run the bench: the real verifier, observed.
 *
 * Never throws for a bad record - a malformed envelope, an unreachable chain and a forged credential
 * are all REPORTED, because "the bench crashed" tells an operator nothing about which check objected.
 */
export async function runVerificationBench(input: BenchInput): Promise<BenchReport> {
  // A JSON primitive, an array or `null` is a record too, and the page's whole purpose is that ANY of
  // them can be thrown at it. Reaching straight for `input.wrappedDoc.issuer` threw a TypeError out of
  // the call, which the contract above forbids and which the page could only render as a dead button.
  // An empty object flows through every check below and is REPORTED - malformed envelope, no verdict.
  const doc = isObject(input.wrappedDoc) ? input.wrappedDoc : {};
  const blockNumber = input.blockNumber ?? null;
  const factoryAddr = input.factoryAddr?.trim() || DEPLOYED_ADDRESSES.DogTagIssuerFactory;
  const registryAddr = input.registryAddr?.trim() || DEPLOYED_ADDRESSES.IssuerRegistry;
  const today = input.today ?? todayIso();

  const issuerMeta = isObject(doc.issuer) ? doc.issuer : {};
  const documentStore = str(issuerMeta.documentStore).trim();
  const claimedDomain = str(issuerMeta.domain).trim();
  const claimedRoot = isObject(doc.signature) ? str(doc.signature.merkleRoot) : "";

  // ── Run the REAL verifier through the recorder ────────────────────────────────────────────────
  const { reader, reads } = recordingReader(
    // The bench's default is the verifier's OWN reader factory, pinned to this run's block - one
    // reader implementation shared, rather than two that could drift.
    input.reader ?? roaxIssuerChainReader(input.rpcUrl, input.blockNumber),
  );
  let response: VerifyCredentialResp | null = null;
  let verifierError: string | null = null;
  try {
    response = await verifyCredentialOnchain({
      wrappedDoc: doc,
      registryAddr,
      factoryAddr,
      rpcUrl: input.rpcUrl,
      reader,
      now: input.now,
    });
  } catch (e) {
    verifierError = e instanceof Error ? e.message : String(e);
  }

  const blockLine: EvidenceLine = {
    label: "Block",
    value: blockNumber === null ? "latest (unanchored - the head could not be pinned)" : blockNumber.toString(),
    source: blockNumber === null ? "offline" : "eth_call blockNumber",
  };

  const checks: Array<Omit<BenchCheck, "gatesVerdict">> = [
    integrityCheck(doc, response),
    ...issuerAnchorChecks(reads, documentStore, claimedRoot, blockLine, verifierError),
    whitelistCheck(reads, response, registryAddr, blockLine, verifierError),
    anchoredCheck(response, reads, verifierError),
    revokedCheck(response, reads, verifierError),
    expiryCheck(doc, today),
  ];

  checks.push(
    ...(await domainChecks(input, reads, claimedDomain, blockLine)),
  );

  return {
    // Stamped centrally so a newly added check cannot silently omit it; `GATES_VERDICT` is exhaustive
    // over `BenchCheckId`, so the compiler requires an answer for every check that exists.
    checks: checks.map((c) => ({ ...c, gatesVerdict: GATES_VERDICT[c.id] })),
    verdict: response ? response.verdict : null,
    response,
    verifierError,
    blockNumber,
    reads,
  };
}

// ── individual checks ───────────────────────────────────────────────────────────────────────────
//
// Each derives its outcome from the REAL verifier's response where the verifier owns the answer, and
// from RECORDED reads where it does not report one. None re-derives a verdict the verifier already gave.

function integrityCheck(
  doc: Record<string, unknown>,
  response: VerifyCredentialResp | null,
): Omit<BenchCheck, "gatesVerdict"> {
  const question = "Is the document's content intact - does it still hash to the root it claims?";
  const claimed = isObject(doc.signature) ? str(doc.signature.merkleRoot) : "";

  // The verifier owns this answer whenever it produced one.
  if (response) {
    const ok = response.fragments.integrity;
    return {
      id: "integrity",
      question,
      outcome: ok ? "pass" : "fail",
      finding: ok
        ? "The recomputed Merkle root matches the root the document claims."
        : "The recomputed Merkle root does NOT match the root the document claims: a covered field was changed after issuance.",
      evidence: [
        { label: "Claimed root", value: response.root, source: "document signature.merkleRoot" },
        { label: "Recomputed root", value: response.recomputedRoot, source: "offline" },
      ],
    };
  }

  // The verifier rejected before returning (an unreachable chain, or a malformed envelope). Integrity
  // is PURE and offline, so it can still be answered - using the very same `checkIntegrity` the
  // verifier calls internally, not a reimplementation of it.
  try {
    const { state, root } = checkIntegrity(doc as unknown as WrappedDoc);
    const ok = state === "VALID";
    return {
      id: "integrity",
      question,
      outcome: ok ? "pass" : "fail",
      finding: ok
        ? "The recomputed Merkle root matches the root the document claims."
        : "The recomputed Merkle root does NOT match the root the document claims.",
      evidence: [
        { label: "Claimed root", value: claimed || "(absent)", source: "document signature.merkleRoot" },
        { label: "Recomputed root", value: toHex32(root), source: "offline" },
      ],
    };
  } catch (e) {
    return {
      id: "integrity",
      question,
      outcome: "could-not-run",
      finding: "The document could not be parsed well enough to recompute its root.",
      couldNotRunReason: `The envelope is malformed: ${e instanceof Error ? e.message : String(e)}`,
      evidence: [{ label: "Claimed root", value: claimed || "(absent)", source: "document signature.merkleRoot" }],
    };
  }
}

/**
 * The factory anchor and the document's agreement with it - TWO checks, deliberately never one.
 *
 * "No factory clone ever issued this root" and "the document names a different contract than the one
 * that did" are different accusations with different remedies, and the backend keeps them apart too
 * (`issuer_mismatch` vs `issuer_unresolved`). Folding them would hide which one actually fired.
 */
function issuerAnchorChecks(
  reads: ChainRead[],
  documentStore: string,
  claimedRoot: string,
  blockLine: EvidenceLine,
  verifierError: string | null,
): Array<Omit<BenchCheck, "gatesVerdict">> {
  const anchorQ = "Was this issued by a contract that genuinely descends from the DogTag factory?";
  const agreeQ = "Does the document name the same contract the factory says issued it?";
  const read = lastRead(reads, "rootIssuer");

  const docLine: EvidenceLine = {
    label: "Contract the document names",
    value: documentStore || "(absent)",
    source: "document issuer.documentStore (NOT covered by the Merkle root)",
  };

  if (!read) {
    const reason = verifierError
      ? `The document was rejected before any chain read was attempted: ${verifierError}`
      : "The factory was never asked.";
    return [
      {
        id: "issuer-descends-from-factory",
        question: anchorQ,
        outcome: "could-not-run",
        finding: "The factory was not consulted for this root.",
        couldNotRunReason: reason,
        evidence: [docLine],
      },
      {
        id: "document-names-issuing-contract",
        question: agreeQ,
        outcome: "could-not-run",
        finding: "There is no authoritative contract to compare the document's claim against.",
        couldNotRunReason: reason,
        evidence: [docLine],
      },
    ];
  }

  // The factory address comes off the READ, not off the config value the caller passed: the two are
  // the same only if the read really went where the caller intended, and that is the fact worth citing.
  // The factory address comes off the READ, not off the config value the caller passed: the two are
  // the same only if the read really went where the caller intended, and that is the fact worth citing.
  const factoryLine: EvidenceLine = {
    label: "Factory asked",
    value: read.contract,
    source: "DogTagIssuerFactory.rootIssuer(root) - this client's own configured factory",
  };
  const rootLine: EvidenceLine = { label: "Root asked about", value: claimedRoot, source: "document signature.merkleRoot" };

  if (read.outcome === "failed") {
    const reason = `The factory's rootIssuer read failed: ${read.error ?? "unknown error"}`;
    return [
      {
        id: "issuer-descends-from-factory",
        question: anchorQ,
        outcome: "could-not-run",
        finding: "The chain could not be asked which contract issued this root.",
        couldNotRunReason: reason,
        evidence: [factoryLine, rootLine, docLine, blockLine],
      },
      {
        id: "document-names-issuing-contract",
        question: agreeQ,
        outcome: "could-not-run",
        finding: "There is no authoritative contract to compare the document's claim against.",
        couldNotRunReason: reason,
        evidence: [docLine, blockLine],
      },
    ];
  }

  const resolved = read.value ?? "";
  if (isZeroAddr(resolved) || !resolved) {
    // We ASKED and the chain has no record. That is evidence ABOUT the credential - a definite
    // failure, not an unanswered question. The codebase draws this line deliberately: `noRecord`
    // gates, `noFactoryConfigured` does not.
    return [
      {
        id: "issuer-descends-from-factory",
        question: anchorQ,
        outcome: "fail",
        finding:
          "The factory has NO record of this root. No contract it deployed ever issued this credential.",
        evidence: [factoryLine, rootLine, docLine, blockLine],
      },
      {
        id: "document-names-issuing-contract",
        question: agreeQ,
        outcome: "could-not-run",
        finding:
          "The factory named no contract, so there is nothing authoritative to compare the document's claim against.",
        couldNotRunReason:
          "The anchor check above already failed; this one needs a resolved contract to disagree with.",
        evidence: [docLine, blockLine],
      },
    ];
  }

  const resolvedLine: EvidenceLine = {
    label: "Contract the factory names",
    value: resolved,
    source: "DogTagIssuerFactory.rootIssuer(root) - write-once, writable only from inside a clone's issue()",
  };
  // A plain comparison of two RECORDED values, not a second implementation of the verifier's rule:
  // the verifier folds this into `issuerWhitelisted`, and what is stated here is only which two
  // addresses were observed and whether they are the same string.
  const agrees = !!documentStore && resolved.toLowerCase() === documentStore.toLowerCase();
  return [
    {
      id: "issuer-descends-from-factory",
      question: anchorQ,
      outcome: "pass",
      finding: "The factory names a contract it deployed as the issuer of this root.",
      evidence: [factoryLine, rootLine, resolvedLine, blockLine],
    },
    {
      id: "document-names-issuing-contract",
      question: agreeQ,
      outcome: agrees ? "pass" : "fail",
      finding: agrees
        ? "The document names the same contract the factory does."
        : "The document names a DIFFERENT contract than the one the factory says issued this root.",
      evidence: [resolvedLine, docLine, blockLine],
    },
  ];
}

function whitelistCheck(
  reads: ChainRead[],
  response: VerifyCredentialResp | null,
  registryAddr: string,
  blockLine: EvidenceLine,
  verifierError: string | null,
): Omit<BenchCheck, "gatesVerdict"> {
  const question = "Was the signer that issued this whitelisted for this record type?";
  const signerRead = lastRead(reads, "issuedBy");
  const rtRead = lastRead(reads, "recordType");
  const wlRead = lastRead(reads, "isWhitelistedFor");

  // The registry line is emitted only when the registry was ACTUALLY consulted. Stating "Registry
  // asked" unconditionally cited a read that, on every unresolved-clone path, was never made.
  const evidence: EvidenceLine[] = [
    wlRead
      ? readEvidence("Registry answer", wlRead, "IssuerRegistry.isWhitelistedFor(recordTypeKey, signer)")
      : {
          label: "Registry",
          value: `not consulted (${registryAddr})`,
          source: "no isWhitelistedFor read was made - see the reason below",
        },
  ];
  if (signerRead) {
    evidence.push(readEvidence("Signer the chain names", signerRead, "DogTagIssuer.issuedBy(root)"));
  }
  if (rtRead) {
    evidence.push(readEvidence("Record type the contract declares", rtRead, "DogTagIssuer.recordType()"));
  }
  evidence.push(blockLine);

  // The verifier owns this answer. Its tri-state is preserved exactly: only a definite `true` passes,
  // a definite `false` fails, and `null` is indeterminate - never promoted to either.
  if (response) {
    const wl = response.fragments.issuerWhitelisted;
    if (wl === true) {
      return {
        id: "issuer-whitelisted",
        question,
        outcome: "pass",
        finding: "The signer the chain says issued this root is whitelisted for the record type that contract declares.",
        evidence,
      };
    }
    if (wl === false) {
      return {
        id: "issuer-whitelisted",
        question,
        outcome: "fail",
        finding:
          "The signer is NOT authorised for this record type - or the document misrepresents the issuing contract or the record type.",
        evidence,
      };
    }
    return {
      id: "issuer-whitelisted",
      question,
      outcome: "could-not-run",
      finding: "The authorising signer could not be established.",
      couldNotRunReason: whitelistUnresolvedReason(reads),
      evidence,
    };
  }

  return {
    id: "issuer-whitelisted",
    question,
    outcome: "could-not-run",
    finding: "The check never completed.",
    couldNotRunReason:
      failedReadReason(reads) ??
      (verifierError ? `The verifier stopped before this check: ${verifierError}` : "The check was not reached."),
    evidence,
  };
}

/** Which missing link left the whitelist pillar indeterminate, stated from the reads actually made. */
function whitelistUnresolvedReason(reads: ChainRead[]): string {
  const failed = failedReadReason(reads);
  if (failed) return failed;
  const anchor = lastRead(reads, "rootIssuer");
  if (!anchor || isZeroAddr(anchor.value) || !anchor.value) {
    return "No factory-deployed contract was resolved for this root, so there is no authoritative issuer to ask the registry about.";
  }
  const rt = lastRead(reads, "recordType");
  if (rt?.outcome === "ok" && /^0x0*$/.test(rt.value ?? "")) {
    return "The issuing contract declares no record type, so the registry cannot be asked the right question.";
  }
  const signer = lastRead(reads, "issuedBy");
  if (signer?.outcome === "ok" && isZeroAddr(signer.value)) {
    return "The contract the factory named never issued this root, so it names no originating signer.";
  }
  return "The issuing signer could not be resolved from the chain.";
}

/** The first failed read, phrased as a reason. `null` when every attempted read succeeded. */
function failedReadReason(reads: ChainRead[]): string | null {
  const f = reads.find((r) => r.outcome === "failed");
  return f ? `The chain read \`${f.method}\` against ${f.contract} failed: ${f.error ?? "unknown error"}` : null;
}

function anchoredCheck(
  response: VerifyCredentialResp | null,
  reads: ChainRead[],
  verifierError: string | null,
): Omit<BenchCheck, "gatesVerdict"> {
  return readBackedCheck({
    id: "anchored-on-chain",
    question: "Is this root actually anchored on-chain by its issuing contract?",
    method: "issuedAt",
    call: "DogTagIssuer.issuedAt(root)",
    label: "issuedAt",
    reads,
    response,
    verifierError,
    unestablished: "The anchoring timestamp was not established.",
    decide: (resp) => ({
      outcome: resp.fragments.issued ? "pass" : "fail",
      finding: resp.fragments.issued
        ? "The issuing contract records an anchoring timestamp for this root."
        : "The issuing contract has never anchored this root (issuedAt is 0).",
    }),
  });
}

function revokedCheck(
  response: VerifyCredentialResp | null,
  reads: ChainRead[],
  verifierError: string | null,
): Omit<BenchCheck, "gatesVerdict"> {
  return readBackedCheck({
    id: "not-revoked",
    question: "Has the issuer revoked this credential?",
    method: "isRevoked",
    call: "DogTagIssuer.isRevoked(root)",
    label: "isRevoked",
    reads,
    response,
    verifierError,
    unestablished: "Revocation status was not established.",
    // The check PASSES when the credential is NOT revoked - the question is phrased as the risk.
    decide: (resp) => ({
      outcome: resp.fragments.revoked ? "fail" : "pass",
      finding: resp.fragments.revoked
        ? "The issuing contract has revoked this root."
        : "The issuing contract has not revoked this root.",
    }),
  });
}

/**
 * The validity window - a PURE offline check with no on-chain counterpart.
 *
 * The chain records anchoring and revocation, not expiry: `validUntil` lives inside the credential and
 * is covered by the Merkle root, so it is tamper-evident but never consulted by any `isValid` read.
 * That is precisely why it needs its own row - an expired credential is on-chain-valid.
 *
 * The comparison is a plain ISO string compare (`today > validUntil`), byte-identical to the backend's
 * `expired_or_valid`. Parsing the dates instead would introduce a second date implementation that could
 * disagree with the Rust side about timezones or leap days.
 *
 * A document with NO window reports `could-not-run`, not `pass`. There is no validity claim to test,
 * and reporting "not expired" would be an assertion about a field the document never made. (The
 * backend's record-status view folds absence into "not expired"; that is a different question - the
 * lifecycle of a stored record, not a check of a presented one.)
 */
function expiryCheck(doc: Record<string, unknown>, today: string): Omit<BenchCheck, "gatesVerdict"> {
  const question = "Is this credential still within its validity window?";
  let found: ValidityWindow | null = null;
  try {
    found = validUntilOf(doc);
  } catch {
    found = null;
  }
  if (!found) {
    return {
      id: "not-expired",
      question,
      outcome: "could-not-run",
      finding: "This document carries no validity window.",
      couldNotRunReason:
        "No `validUntil` field is present in the Merkle-covered data, so there is no expiry claim to test.",
      evidence: [{ label: "Today (UTC)", value: today, source: "offline" }],
    };
  }
  if (!found.usable) {
    // A blank or non-ISO leaf is UNREADABLE, not lapsed. `today > ""` is true for every date, so
    // comparing it would accuse a credential whose window was simply left empty at issuance - the same
    // could-not-run-rendered-as-an-answer collapse the absent case above already refuses.
    return {
      id: "not-expired",
      question,
      outcome: "could-not-run",
      finding: "This document's validity window is present but unreadable.",
      couldNotRunReason: `The \`${found.keyPath}\` leaf carries ${
        found.value ? `\`${found.value}\`, which is not an ISO YYYY-MM-DD date` : "no value at all"
      }, so there is no window this check can compare today against.`,
      evidence: [
        { label: "validUntil", value: found.value || "(blank)", source: `document data.${found.keyPath}` },
        { label: "Today (UTC)", value: today, source: "offline" },
      ],
    };
  }
  const expired = today > found.value;
  return {
    id: "not-expired",
    question,
    outcome: expired ? "fail" : "pass",
    finding: expired
      ? `The validity window lapsed on ${found.value}.`
      : `Valid until ${found.value}.`,
    evidence: [
      { label: "validUntil", value: found.value, source: `document data.${found.keyPath} (covered by the Merkle root)` },
      { label: "Today (UTC)", value: today, source: "offline" },
    ],
  };
}

/**
 * The issuer's domain, as TWO checks - the on-chain claim, and the DNS zone that must corroborate it.
 *
 * They are separate rows because only the first is answerable from a browser. The DNS half is resolved
 * server-side (`dogtag-dns-rs`), and a bench that showed one green row for "domain verified" would let
 * a reader believe a TXT lookup happened that never did.
 */
async function domainChecks(
  input: BenchInput,
  reads: ChainRead[],
  claimedDomain: string,
  blockLine: EvidenceLine,
): Promise<Array<Omit<BenchCheck, "gatesVerdict">>> {
  // NOTE the absent parameter: the document's own `documentStore` is deliberately NOT in scope here.
  // The domain claim is read from the clone the FACTORY named, and having the document's address
  // available would make reading the claim off it a one-character mistake.
  const claimQ = "Does the domain the document claims match the one the issuer published on-chain?";
  const dnsQ = "Does that domain's DNS zone name this issuing contract back?";
  const docLine: EvidenceLine = {
    label: "Domain the document claims",
    value: claimedDomain || "(absent)",
    source: "document issuer.domain (NOT covered by the Merkle root)",
  };

  // The DNS half is never performed here. It reports could-not-run ALWAYS, with the reason, rather
  // than being omitted - a check a surface cannot make is exactly what this bench exists to say aloud.
  const dnsCheck: Omit<BenchCheck, "gatesVerdict"> = {
    id: "issuer-domain-dns",
    question: dnsQ,
    outcome: "could-not-run",
    finding: "No DNS lookup was performed.",
    couldNotRunReason:
      "The TXT lookup is resolved server-side by the verifier backends; this bench runs in the browser and cannot perform it. A passing on-chain claim above is NOT evidence that DNS agrees.",
    evidence: [docLine],
  };

  const registry = input.domainRegistryAddr?.trim();
  if (!registry) {
    return [
      {
        id: "issuer-domain-claim",
        question: claimQ,
        outcome: "could-not-run",
        finding: "No issuer-domain registry is configured for this bench.",
        couldNotRunReason:
          "No IssuerDomainRegistry address is configured, so the issuer's own on-chain domain claim could not be read.",
        evidence: [docLine],
      },
      dnsCheck,
    ];
  }

  const anchor = lastRead(reads, "rootIssuer");
  const clone = anchor?.outcome === "ok" && !isZeroAddr(anchor.value) ? anchor.value : undefined;
  if (!clone) {
    return [
      {
        id: "issuer-domain-claim",
        question: claimQ,
        outcome: "could-not-run",
        finding: "There is no factory-resolved contract whose domain claim could be read.",
        couldNotRunReason:
          "The domain claim is read from the contract the FACTORY named, never from the document. No such contract was resolved, and reading the claim off the address the document names would let a forged document answer a question about itself.",
        evidence: [docLine],
      },
      dnsCheck,
    ];
  }

  const claimReader: DomainClaimReader = input.domainClaimReader ?? {
    claim: (registryAddr, cloneAddr) =>
      issuerDomainClaimOf({
        domainRegistryAddr: registryAddr,
        cloneAddr,
        rpcUrl: input.rpcUrl,
        blockNumber: input.blockNumber,
      }),
  };

  let claim: { domain: string; updatedAtBlock: bigint } | null = null;
  try {
    claim = await claimReader.claim(registry, clone);
    reads.push({
      method: "issuerDomainClaim",
      contract: registry,
      args: [clone],
      outcome: "ok",
      value: claim ? claim.domain : "(no claim published)",
    });
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e);
    reads.push({ method: "issuerDomainClaim", contract: registry, args: [clone], outcome: "failed", error });
    return [
      {
        id: "issuer-domain-claim",
        question: claimQ,
        outcome: "could-not-run",
        finding: "The issuer's on-chain domain claim could not be read.",
        couldNotRunReason: `The IssuerDomainRegistry read failed: ${error}`,
        evidence: [docLine, blockLine],
      },
      dnsCheck,
    ];
  }

  if (!claim) {
    // "This issuer has published no domain claim" is the NORMAL day-one state. Neither a pass nor a
    // failure - there is simply nothing to compare the document's claim against.
    return [
      {
        id: "issuer-domain-claim",
        question: claimQ,
        outcome: "could-not-run",
        finding: "This issuer has published no domain claim.",
        couldNotRunReason:
          "The IssuerDomainRegistry holds no binding for this contract, so the document's claimed domain cannot be corroborated or contradicted.",
        evidence: [
          docLine,
          { label: "Registry asked", value: registry, source: `IssuerDomainRegistry.getBinding(${clone})` },
          blockLine,
        ],
      },
      dnsCheck,
    ];
  }

  const matches = !!claimedDomain && claim.domain.toLowerCase() === claimedDomain.toLowerCase();
  return [
    {
      id: "issuer-domain-claim",
      question: claimQ,
      outcome: matches ? "pass" : "fail",
      finding: matches
        ? "The document's claimed domain matches the issuer's own on-chain claim."
        : "The document claims a DIFFERENT domain than the issuing contract published on-chain.",
      evidence: [
        { label: "Domain the issuer published", value: claim.domain, source: `IssuerDomainRegistry.getBinding(${clone})` },
        docLine,
        { label: "Claim written at block", value: claim.updatedAtBlock.toString(), source: "IssuerDomainRegistry binding" },
        blockLine,
      ],
    },
    dnsCheck,
  ];
}
