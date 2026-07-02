// PRESENT — the holder side of the on-chain proof-of-verification, reproduced in the browser. This
// is the "phone ZK" step the native apps do on-device, done here client-side against the owner's
// trusted prover + the verifier they scanned. The genuine client-side crypto (the §1.10 consent build
// and the EdDSA-BabyJubjub consent signature) runs IN THE BROWSER via @dogtag/standard; only the heavy
// Groth16 witness/proof is delegated to the trusted prover-service (`POST /prove-verification`), which
// is the same "owner's own prover" model as the native 32-bit fallback. The verifier only ever sees
// the resulting proof — never the witness.
//
// The wire shapes here match the live vet/groomer backend exactly:
//   GET  <verifier>/x/<token>              -> { sessionId, relayer, purpose, recordType, challenge, mode }
//   POST <prover>/prove-verification       <- { wrappedDoc, consent, eddsaSig } -> { a, b, c, pub }
//   POST <verifier>/verify/consent/submit  <- { exportToken, consent, sig, mode, proof, bind }
//   GET  <verifier>/verify/session/<id>?token=<token> -> { status, txHash, nullifier }
import {
  FIELD_P,
  signConsentEddsa,
  type VerificationConsent,
  type WrappedDoc,
} from "@dogtag/standard";
import { keccak256, toBytes } from "viem";
import { CONTRACTS, ROAX_CHAIN_ID } from "./config";
import { readBindNonce } from "./chain";
import { summarize } from "./credential";
import type { OwnerWallet } from "./wallet";

export type PresentStep =
  | "resolving"
  | "signing"
  | "proving"
  | "binding"
  | "submitting"
  | "polling"
  | "verified"
  | "failed";

export interface PresentProgress {
  step: PresentStep;
  note?: string;
}

export interface VerifySession {
  sessionId: string;
  relayer: string;
  purpose: string;
  recordType: string;
  challenge: string;
  mode: string;
}

export interface PresentResult {
  txHash: string | null;
  nullifier: string | null;
  session: VerifySession;
}

export interface PresentOptions {
  /** the verify link scanned from a verifier — `<verifier>/x/<token>?a=<relayer>`. */
  link: string;
  wallet: OwnerWallet;
  doc: WrappedDoc;
  proverUrl: string;
  onProgress?: (p: PresentProgress) => void;
  /** injectable fetch (tests) + poll cadence/cap; defaults are demo-friendly. */
  fetchImpl?: typeof fetch;
  pollIntervalMs?: number;
  pollTimeoutMs?: number;
  /** aborts in-flight fetches and stops the poll loop (e.g. on page unmount). */
  signal?: AbortSignal;
}

/** A field element (bigint) -> 0x + 64-hex big-endian word. */
function word32Hex(x: bigint): string {
  return "0x" + (x % (1n << 256n)).toString(16).padStart(64, "0");
}

/** An address -> its canonical 20-byte lowercase 0x hex (defensive against checksum/space). */
function addr20(a: string): string {
  const s = a.trim().toLowerCase();
  const hex = s.startsWith("0x") ? s.slice(2) : s;
  if (hex.length !== 40) throw new Error(`bad address: ${a}`);
  return "0x" + hex;
}

/** purpose_key(label) = keccak256(label) reduced mod BN254 r — the field-element purpose. */
function purposeField(label: string): bigint {
  return BigInt(keccak256(toBytes(label))) % FIELD_P;
}

/** recordType binding = keccak256(recordType) (full bytes32, NOT reduced). */
function recordTypeWord(label: string): string {
  return keccak256(toBytes(label));
}

/** A fresh 128-bit consent nonce (uniqueness -> a fresh nullifier per presentation). */
function freshNonce(): bigint {
  const b = new Uint8Array(16);
  crypto.getRandomValues(b);
  let v = 0n;
  for (const x of b) v = (v << 8n) | BigInt(x);
  return v;
}

/** Serialize a VerificationConsent to the all-0x-hex JSON the prover + verifier parse. */
function consentToHexJson(c: VerificationConsent): Record<string, string> {
  return {
    dogTagId: word32Hex(c.dogTagId),
    recordType: c.recordType,
    purpose: c.purpose,
    credentialRoot: c.credentialRoot,
    challenge: c.challenge,
    relayer: addr20(c.relayer),
    subject: addr20(c.subject),
    nonce: word32Hex(c.nonce),
    deadline: word32Hex(c.deadline),
  };
}

/** The owner's EIP-712 BindConsentKey signature — authorizes a gasless relayer bind of the key. */
async function signBind(wallet: OwnerWallet, nonce: bigint): Promise<string> {
  return wallet.account.signTypedData({
    domain: {
      name: "DogTag",
      version: "1",
      chainId: ROAX_CHAIN_ID,
      verifyingContract: CONTRACTS.ConsentKeyRegistry as `0x${string}`,
    },
    types: {
      BindConsentKey: [
        { name: "babyJubPubKeyHash", type: "bytes32" },
        { name: "wallet", type: "address" },
        { name: "nonce", type: "uint256" },
      ],
    },
    primaryType: "BindConsentKey",
    message: {
      babyJubPubKeyHash: wallet.keyHash as `0x${string}`,
      wallet: wallet.address,
      nonce,
    },
  });
}

interface ProofResponse {
  a: string[];
  b: string[][];
  c: string[];
  pub: string[];
}

/** Parse the `<verifier>/x/<token>` link into its base origin + one-time token. */
export function parseVerifyLink(link: string): { base: string; token: string } {
  let url: URL;
  try {
    url = new URL(link.trim());
  } catch {
    throw new Error("Paste the full verify link (starts with http…/x/…) shown by the verifier.");
  }
  const m = url.pathname.match(/\/x\/([^/]+)$/);
  if (!m) throw new Error("That link is not a verify link (expected …/x/<token>).");
  return { base: url.origin, token: m[1]! };
}

/**
 * Run the full present loop. Emits progress at each step and resolves with the on-chain result once
 * the verifier records the verification. Throws (with a human message) at the first failing step.
 */
export async function presentCredential(opts: PresentOptions): Promise<PresentResult> {
  const {
    link,
    wallet,
    doc,
    proverUrl,
    onProgress = () => {},
    fetchImpl = fetch,
    pollIntervalMs = 3000,
    pollTimeoutMs = 150_000,
    signal,
  } = opts;
  const { base, token } = parseVerifyLink(link);

  // 1. Resolve the verify session the owner scanned.
  onProgress({ step: "resolving", note: "Reading the verifier's request…" });
  const sres = await fetchImpl(`${base}/x/${token}`, { signal });
  if (!sres.ok) throw new Error(`Verify link expired or invalid (HTTP ${sres.status}).`);
  const session = (await sres.json()) as VerifySession;

  // 2. Build the §1.10 consent binding this credential to the verifier's request.
  const summary = summarize(doc);
  if (!summary.dogTagId) throw new Error("This credential has no dogTagId and cannot be presented.");
  const nonce = freshNonce();
  const deadline = BigInt(Math.floor(Date.now() / 1000) + 3600);
  const consent: VerificationConsent = {
    dogTagId: BigInt(summary.dogTagId),
    recordType: recordTypeWord(session.recordType),
    purpose: word32Hex(purposeField(session.purpose)),
    credentialRoot: doc.signature.merkleRoot,
    challenge: session.challenge,
    relayer: session.relayer,
    subject: wallet.address,
    nonce,
    deadline,
  };

  // 3. EdDSA-BabyJubjub sign the consent IN THE BROWSER (the sensitive "phone ZK" client step).
  onProgress({ step: "signing", note: "Signing consent with your on-device key…" });
  const eddsa = await signConsentEddsa(consent, wallet.consentPrv);
  const consentHex = consentToHexJson(consent);
  const eddsaSig = {
    r8xDec: eddsa.R8x,
    r8yDec: eddsa.R8y,
    sDec: eddsa.S,
    axHex: word32Hex(wallet.Ax),
    ayHex: word32Hex(wallet.Ay),
  };

  // 4. Delegate the heavy Groth16 proof to the owner's trusted prover-service.
  onProgress({ step: "proving", note: "Generating the zero-knowledge proof…" });
  const pres = await fetchImpl(`${proverUrl}/prove-verification`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ wrappedDoc: doc, consent: consentHex, eddsaSig }),
    signal,
  });
  if (!pres.ok) {
    throw new Error(`Prover failed (HTTP ${pres.status}). Check your prover-service URL.`);
  }
  const proof = (await pres.json()) as ProofResponse;

  // 5. Authorize the gasless consent-key bind (needed the first time this wallet presents a ZK proof).
  onProgress({ step: "binding", note: "Authorizing gasless consent-key bind…" });
  const bindNonce = await readBindNonce(wallet.address);
  const ownerSig = await signBind(wallet, bindNonce);
  const bind = { subject: wallet.address, keyHash: wallet.keyHash, ownerSig };

  // 6. Submit the consent + proof to the verifier; it relays the on-chain record as the gas payer.
  onProgress({ step: "submitting", note: "Submitting proof to the verifier…" });
  const subRes = await fetchImpl(`${base}/verify/consent/submit`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      exportToken: token,
      consent: consentHex,
      sig: "",
      mode: "zk",
      proof: { a: proof.a, b: proof.b, c: proof.c, pubSignals: proof.pub },
      bind,
    }),
    signal,
  });
  if (!subRes.ok) {
    let msg = `Verifier rejected the submission (HTTP ${subRes.status}).`;
    try {
      const j = (await subRes.json()) as { error?: string };
      if (j.error) msg = j.error;
    } catch {
      /* keep the generic message */
    }
    throw new Error(msg);
  }

  // 7. Poll the session until the verifier records it on chain (or reports a failure).
  onProgress({ step: "polling", note: "Waiting for on-chain confirmation…" });
  const deadlineAt = Date.now() + pollTimeoutMs;
  for (;;) {
    signal?.throwIfAborted();
    const stat = await fetchImpl(`${base}/verify/session/${session.sessionId}?token=${token}`, { signal });
    if (stat.ok) {
      const s = (await stat.json()) as { status: string; txHash?: string | null; nullifier?: string | null };
      if (s.status === "recorded") {
        onProgress({ step: "verified" });
        return { txHash: s.txHash ?? null, nullifier: s.nullifier ?? null, session };
      }
      if (s.status === "error") {
        throw new Error(s.txHash || "The verifier could not record the verification.");
      }
    }
    if (Date.now() >= deadlineAt) throw new Error("Timed out waiting for on-chain confirmation.");
    await sleep(pollIntervalMs, signal);
  }
}

/** A cancellable delay: resolves after `ms`, or rejects immediately if `signal` aborts. */
function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal?.reason ?? new DOMException("Aborted", "AbortError"));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}
