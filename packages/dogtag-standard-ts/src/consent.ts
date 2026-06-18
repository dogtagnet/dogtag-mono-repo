// DogTag consent module — on-chain proof-of-verification consent artifact (impl §11.8/§11.9, §1.10).
//
// Three independent commitments over the SAME VerificationConsent, byte-for-byte equal to
// crates/dogtag-standard-rs/src/consent.rs:
//   (1) EIP-712 typed-data digest (ECDSA, signed by the wallet — keccak256 / secp256k1).
//   (2) Poseidon nullifier = Poseidon(DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce).
//   (3) EdDSA-BabyJubjub consent message M = Poseidon(dogTagId, purpose, relayer, subject, R, nonce).
// keyHash = Poseidon(Ax, Ay).
//
// The EIP-712 leg uses keccak256 (NOT Poseidon) per EIP-712; Poseidon is BN254-only.
import {keccak_256} from "@noble/hashes/sha3";
// circomlibjs is plain JS (no bundled types) — declared ambiently in ./circomlibjs.d.ts.
import {buildEddsa, buildBabyjub} from "circomlibjs";
import {poseidon2, poseidon6} from "poseidon-lite";
import {DS_NULLIFIER, FIELD_P} from "./field.js";

// ---------------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------------

/**
 * The FINAL on-chain VerificationConsent (impl §11.9(a)) — NINE fields in this exact order.
 * uint256 fields (dogTagId/nonce/deadline) are bigints; bytes32 (recordType/purpose/credentialRoot/
 * challenge) and address (relayer/subject) are 0x-hex strings.
 */
export interface VerificationConsent {
  dogTagId: bigint;
  recordType: string; // bytes32
  purpose: string; // bytes32
  credentialRoot: string; // bytes32 (the credential root R)
  challenge: string; // bytes32
  relayer: string; // address (0x + 40 hex)
  subject: string; // address (0x + 40 hex)
  nonce: bigint;
  deadline: bigint;
}

// ---------------------------------------------------------------------------------------------
// EIP-712 typed-data digest (impl §11.8) — keccak256, NOT Poseidon.
// ---------------------------------------------------------------------------------------------

/** The default EIP-712 chainId (impl §11.8(a)). */
export const DOGTAG_CHAIN_ID = 135n;

/** EIP-712 type string — field order MUST match the struct (impl §11.8(a)). */
export const VERIFICATION_CONSENT_TYPE_STRING =
  "VerificationConsent(uint256 dogTagId,bytes32 recordType,bytes32 purpose,bytes32 credentialRoot," +
  "bytes32 challenge,address relayer,address subject,uint256 nonce,uint256 deadline)";

/** The EIP-712 EIP712Domain type string. */
const EIP712_DOMAIN_TYPE_STRING =
  "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

const utf8 = new TextEncoder();

function keccak(bytes: Uint8Array): Uint8Array {
  return keccak_256(bytes);
}

function toHex(bytes: Uint8Array): string {
  let h = "0x";
  for (const b of bytes) h += b.toString(16).padStart(2, "0");
  return h;
}

function hexToBytes(h: string): Uint8Array {
  const s = h.startsWith("0x") ? h.slice(2) : h;
  if (s.length % 2 !== 0) throw new Error(`odd hex length: ${h}`);
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** Left-pad a bigint to a 32-byte big-endian word. */
function word32(x: bigint): Uint8Array {
  if (x < 0n) throw new Error("negative word");
  const out = new Uint8Array(32);
  let v = x;
  for (let i = 31; i >= 0 && v > 0n; i--) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  if (v > 0n) throw new Error("word exceeds 32 bytes");
  return out;
}

/** A bytes32 hex (already 32 bytes) -> 32-byte word; rejects over-long. */
function bytes32Word(h: string): Uint8Array {
  const b = hexToBytes(h);
  if (b.length > 32) throw new Error(`bytes32 too long: ${h}`);
  const out = new Uint8Array(32);
  out.set(b, 32 - b.length); // left-pad (right-aligned), matching abi.encode of bytes32 as-is
  return out;
}

/** An address (20 bytes) -> 32-byte left-padded word (abi.encode of address). */
function addressWord(h: string): Uint8Array {
  const b = hexToBytes(h);
  if (b.length > 20) throw new Error(`address too long: ${h}`);
  const out = new Uint8Array(32);
  out.set(b, 32 - b.length);
  return out;
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  let len = 0;
  for (const p of parts) len += p.length;
  const out = new Uint8Array(len);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

/** keccak256 of the EIP-712 type string (impl §11.8(a)). */
export const VERIFICATION_CONSENT_TYPEHASH: string = toHex(
  keccak(utf8.encode(VERIFICATION_CONSENT_TYPE_STRING)),
);

/**
 * EIP-712 domainSeparator for the DogTag domain (impl §11.8(a)):
 * keccak256(abi.encode(EIP712Domain typehash, keccak(name), keccak(version), chainId, verifyingContract)).
 */
export function domainSeparator(verifyingContract: string, chainId: bigint = DOGTAG_CHAIN_ID): string {
  const domainTypehash = keccak(utf8.encode(EIP712_DOMAIN_TYPE_STRING));
  const nameHash = keccak(utf8.encode("DogTag"));
  const versionHash = keccak(utf8.encode("1"));
  const encoded = concatBytes(
    domainTypehash,
    nameHash,
    versionHash,
    word32(chainId),
    addressWord(verifyingContract),
  );
  return toHex(keccak(encoded));
}

/** keccak256(abi.encode(typehash, ...9 fields...)) — the EIP-712 struct hash (impl §11.9(a)). */
export function structHash(consent: VerificationConsent): string {
  const encoded = concatBytes(
    hexToBytes(VERIFICATION_CONSENT_TYPEHASH),
    word32(consent.dogTagId),
    bytes32Word(consent.recordType),
    bytes32Word(consent.purpose),
    bytes32Word(consent.credentialRoot),
    bytes32Word(consent.challenge),
    addressWord(consent.relayer),
    addressWord(consent.subject),
    word32(consent.nonce),
    word32(consent.deadline),
  );
  return toHex(keccak(encoded));
}

/**
 * The EIP-712 digest (the `_hashTypedDataV4` equivalent, impl §11.8):
 * keccak256(0x1901 ++ domainSeparator ++ structHash). The wallet ECDSA-signs THIS.
 */
export function hashTypedConsent(
  consent: VerificationConsent,
  verifyingContract: string,
  chainId: bigint = DOGTAG_CHAIN_ID,
): string {
  const ds = hexToBytes(domainSeparator(verifyingContract, chainId));
  const sh = hexToBytes(structHash(consent));
  const digest = keccak(concatBytes(new Uint8Array([0x19, 0x01]), ds, sh));
  return toHex(digest);
}

// ---------------------------------------------------------------------------------------------
// Poseidon nullifier (impl §11.9(b)) — Poseidon6 over BN254 Fr.
// ---------------------------------------------------------------------------------------------

/** uint160 field element of an address (mod r is a no-op since 2^160 < r). */
function addressField(h: string): bigint {
  const v = BigInt(h.startsWith("0x") ? h : "0x" + h);
  if (v >= 1n << 160n) throw new Error(`address exceeds uint160: ${h}`);
  return v;
}

/** A bytes32 reduced into [0, r) (purpose / roots are BN254-bound). */
function bytes32Field(h: string): bigint {
  const v = BigInt(h.startsWith("0x") ? h : "0x" + h);
  return v % FIELD_P;
}

/**
 * The consent nullifier (impl §11.9(b)):
 * Poseidon(DS_NULLIFIER=4, dogTagId, purpose mod r, uint160(relayer), uint160(subject), nonce).
 * Returns the canonical 32-byte big-endian hex of the BN254 field element.
 */
export function consentNullifier(consent: VerificationConsent): string {
  const m = poseidon6([
    DS_NULLIFIER,
    consent.dogTagId % FIELD_P,
    bytes32Field(consent.purpose),
    addressField(consent.relayer),
    addressField(consent.subject),
    consent.nonce % FIELD_P,
  ]);
  return "0x" + m.toString(16).padStart(64, "0");
}

// ---------------------------------------------------------------------------------------------
// EdDSA-BabyJubjub consent message (impl §11.9(d) / §1.10) — Poseidon6, NO domain tag.
// ---------------------------------------------------------------------------------------------

/**
 * The EdDSA consent message M (impl §1.10):
 * Poseidon(dogTagId, purpose, relayer, subject, credentialRoot(=R), nonce) — 6 inputs, NO DS tag.
 * Returned as a bigint field element so a circuit / EdDSA signer can consume it directly.
 */
export function eddsaConsentMessage(consent: VerificationConsent): bigint {
  return poseidon6([
    consent.dogTagId % FIELD_P,
    bytes32Field(consent.purpose),
    addressField(consent.relayer),
    addressField(consent.subject),
    bytes32Field(consent.credentialRoot),
    consent.nonce % FIELD_P,
  ]);
}

/** keyHash = Poseidon(Ax, Ay) -> canonical 32-byte big-endian hex (impl §1.10). */
export function keyHash(Ax: bigint, Ay: bigint): string {
  const h = poseidon2([Ax % FIELD_P, Ay % FIELD_P]);
  return "0x" + h.toString(16).padStart(64, "0");
}

// ---------------------------------------------------------------------------------------------
// EdDSA-BabyJubjub signing (circomlibjs) — TS only; Rust signing is deferred to the mobile phase.
// ---------------------------------------------------------------------------------------------

/** A derived BabyJubjub consent key: private scalar bytes + public point (Ax, Ay) as bigints. */
export interface BabyjubConsentKey {
  prv: Uint8Array;
  Ax: bigint;
  Ay: bigint;
}

/** An EdDSA-BabyJubjub Poseidon signature (R8 point + scalar S) as decimal strings. */
export interface EddsaConsentSignature {
  R8x: string;
  R8y: string;
  S: string;
}

/**
 * Derive a BabyJubjub consent keypair from a 32-byte seed (circomlibjs EdDSA private key).
 * Returns the private-key bytes plus the public point A = (Ax, Ay) as field bigints.
 */
export async function deriveBabyjubConsentKey(seed: Uint8Array): Promise<BabyjubConsentKey> {
  const eddsa = await buildEddsa();
  const babyjub = await buildBabyjub();
  const F = babyjub.F;
  const prv = seed.slice(); // circomlibjs uses the raw 32-byte buffer as the private key
  const pub = eddsa.prv2pub(prv);
  const Ax = BigInt(F.toString(pub[0]));
  const Ay = BigInt(F.toString(pub[1]));
  return {prv, Ax, Ay};
}

/**
 * EdDSA-BabyJubjub Poseidon signature over the consent message M (impl §11.9(d)).
 * The message is `eddsaConsentMessage(consent)`; signing uses circomlibjs `signPoseidon`.
 */
export async function signConsentEddsa(
  consent: VerificationConsent,
  babyJubPrivKey: Uint8Array,
): Promise<EddsaConsentSignature> {
  const eddsa = await buildEddsa();
  const F = eddsa.F;
  const m = eddsaConsentMessage(consent);
  const msgF = F.e(m.toString());
  const sig = eddsa.signPoseidon(babyJubPrivKey, msgF);
  return {
    R8x: BigInt(F.toString(sig.R8[0])).toString(),
    R8y: BigInt(F.toString(sig.R8[1])).toString(),
    S: BigInt(sig.S).toString(),
  };
}
