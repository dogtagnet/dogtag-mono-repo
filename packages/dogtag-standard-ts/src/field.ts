// Pinned circomlib BN254 Poseidon + byte->field packing (architecture §3.3/§3.4, impl §11.2).
// TS leg pins `poseidon-lite`. Field elements are JS bigints in [0, P); the canonical
// serialization is 32-byte big-endian hex. Byte-for-byte equivalent to dogtag-standard-rs.
import {poseidon2, poseidon3, poseidon5, poseidon6} from "poseidon-lite";

/** BN254 scalar field r (the SNARK scalar field — all reductions pin to this, NOT base q). */
export const FIELD_P = 21888242871839275222246405745257275088548364400416034343698204186575808495617n;

// Domain-separation tags — used as the first input slot, never a capacity IV (impl §1.2).
export const DS_LEAF = 1n;
export const DS_NODE = 2n;
export const DS_BYTES = 3n;
export const DS_NULLIFIER = 4n;

export type Field = bigint;

/** Pinned circomlib Poseidon dispatched by arity (number of inputs). */
export function poseidon(inputs: Field[]): Field {
  switch (inputs.length) {
    case 2:
      return poseidon2(inputs);
    case 3:
      return poseidon3(inputs);
    case 5:
      return poseidon5(inputs);
    case 6:
      return poseidon6(inputs);
    default:
      throw new Error(`unsupported Poseidon arity ${inputs.length} (DogTag uses 2,3,5,6)`);
  }
}

/** Big-endian decode of a byte array into a bigint. */
export function beToBigInt(bytes: Uint8Array): bigint {
  let acc = 0n;
  for (const b of bytes) acc = (acc << 8n) | BigInt(b);
  return acc;
}

/** 8-byte big-endian length prefix. */
function u64be(n: number): Uint8Array {
  const out = new Uint8Array(8);
  let v = BigInt(n);
  for (let i = 7; i >= 0; i--) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

/**
 * Inject a byte string into one field via the length-prefixed, 31-byte-chunked, domain-separated
 * fold (impl §1.2). Each 31-byte limb is < 2^248 < P so the be-decode is injective (no wraparound).
 * Used for variable-length components: keyPath and value.
 */
export function bytesToField(x: Uint8Array): Field {
  const b = new Uint8Array(8 + x.length);
  b.set(u64be(x.length), 0);
  b.set(x, 8);
  let acc = DS_BYTES;
  for (let off = 0; off < b.length; off += 31) {
    // last limb right-zero-padded to 31 bytes
    const limb = new Uint8Array(31);
    const slice = b.subarray(off, off + 31);
    limb.set(slice, 0);
    acc = poseidon([acc, beToBigInt(limb)]);
  }
  // empty input still has the 8-byte length prefix -> at least one limb, so acc != DS_BYTES.
  return acc;
}

/**
 * Pack bytes that fit in a single field directly (<= 31 bytes), big-endian (impl §11.2(a)):
 * salt(16B), addresses(uint160). NEVER folds — these are scalars, not variable-length strings.
 */
export function fieldFromScalarBytes(x: Uint8Array): Field {
  if (x.length > 31) throw new Error(`scalar bytes must be <= 31 (got ${x.length})`);
  return beToBigInt(x);
}

/** A small unsigned integer (typeTag, indices) reduced into [0, P). */
export function fieldFromUint(n: bigint | number): Field {
  const v = BigInt(n);
  if (v < 0n) throw new Error("fieldFromUint: negative");
  return v % FIELD_P;
}

/** Canonical 32-byte big-endian hex (0x-prefixed) of a field element. */
export function toHex32(x: Field): string {
  if (x < 0n || x >= FIELD_P) throw new Error("field element out of range");
  return "0x" + x.toString(16).padStart(64, "0");
}

/** Parse a 0x.. 32-byte hex back into a field element. */
export function fromHex32(h: string): Field {
  const s = h.startsWith("0x") ? h.slice(2) : h;
  const v = BigInt("0x" + s);
  if (v >= FIELD_P) throw new Error("hex exceeds field");
  return v;
}
