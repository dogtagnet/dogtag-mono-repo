// DogTag consent-key module (impl §1.10) — the BabyJubjub consent key material the owner-hidden
// profile tree commits.
//
// `keyHash = Poseidon(Ax, Ay)` is the value the `owner.consentKey` reserved leaf folds into the
// profile root `R`; byte-for-byte equal to `crates/dogtag-standard-rs/src/eddsa.rs::key_hash`
// (cross-language parity is asserted via consent-vectors.json). Consent SIGNING itself happens
// on-device (the mobile FFI) — the web wallet only derives the keypair and its keyHash.
//
// circomlibjs is plain JS (no bundled types) — declared ambiently in ./circomlibjs.d.ts.
import {buildEddsa, buildBabyjub} from "circomlibjs";
import {poseidon2} from "poseidon-lite";
import {FIELD_P} from "./field.js";

/** keyHash = Poseidon(Ax, Ay) -> canonical 32-byte big-endian hex (impl §1.10). */
export function keyHash(Ax: bigint, Ay: bigint): string {
  const h = poseidon2([Ax % FIELD_P, Ay % FIELD_P]);
  return "0x" + h.toString(16).padStart(64, "0");
}

/** A derived BabyJubjub consent key: private scalar bytes + public point (Ax, Ay) as bigints. */
export interface BabyjubConsentKey {
  prv: Uint8Array;
  Ax: bigint;
  Ay: bigint;
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
