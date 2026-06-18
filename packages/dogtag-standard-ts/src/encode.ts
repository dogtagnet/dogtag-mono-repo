// Canonical value encoding (impl §1.1, §11.2). encodeValue is REUSED VERBATIM under Poseidon —
// only the final hash changed; the canonical bytes are identical to the prior keccak spec.
import {TypeTag, type TypedScalar} from "./types.js";

/**
 * Pinned Unicode version for NFC normalization (A3). Node's `String.prototype.normalize` uses the
 * ICU bundled with the runtime; CI pins the Node version so the Unicode data is fixed. Stated here
 * so the Rust SDK (`unicode-normalization`) targets the same Unicode major version.
 */
export const UNICODE_VERSION = "15.1";

const utf8 = new TextEncoder();

/** NFC-normalize and reject unpaired surrogates (A3). */
export function nfc(s: string): string {
  // Reject lone surrogates (ill-formed UTF-16) before normalization.
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c >= 0xd800 && c <= 0xdbff) {
      const next = s.charCodeAt(i + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) throw new Error("unpaired high surrogate");
      i++;
    } else if (c >= 0xdc00 && c <= 0xdfff) {
      throw new Error("unpaired low surrogate");
    }
  }
  return s.normalize("NFC");
}

const INTEGER_RE = /^-?(0|[1-9][0-9]*)$/;
const DECIMAL_RE = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?$/;

/** Canonical integer string: no leading zeros, "-0" -> "0" (A1). */
export function canonicalInteger(s: string): string {
  if (!INTEGER_RE.test(s)) throw new Error(`invalid integer: ${JSON.stringify(s)}`);
  if (s === "-0") return "0";
  return s;
}

/**
 * Canonical decimal string over the INPUT STRING, never a float (A1/A2):
 * strip fractional trailing zeros, drop a trailing ".", map "-0" -> "0",
 * reject exponent/whitespace/"+".
 */
export function canonicalDecimal(s: string): string {
  if (!DECIMAL_RE.test(s)) throw new Error(`invalid decimal: ${JSON.stringify(s)}`);
  let out = s;
  if (out.includes(".")) {
    out = out.replace(/0+$/, ""); // strip fractional trailing zeros
    if (out.endsWith(".")) out = out.slice(0, -1); // drop trailing dot
  }
  if (out === "-0" || out === "-0.0") out = "0";
  // normalize a "-0" that survived (e.g. "-0.000" -> "-0" -> "0")
  if (/^-0$/.test(out)) out = "0";
  return out;
}

/** Hard guard: a native float must never reach the wrap boundary (A2). */
export function assertNotFloat(v: unknown): void {
  if (typeof v === "number" && !Number.isInteger(v)) {
    throw new Error("floats forbidden; pass INTEGER or DECIMAL as a string");
  }
  if (typeof v === "number") {
    throw new Error("native numbers forbidden; pass typed strings");
  }
}

/** encodeValue(typeTag, value) -> canonical bytes (impl §1.1). */
export function encodeValue(s: TypedScalar): Uint8Array {
  switch (s.tag) {
    case TypeTag.Null:
      return new Uint8Array(0);
    case TypeTag.Bool:
      return new Uint8Array([s.value ? 0x01 : 0x00]);
    case TypeTag.String:
      return utf8.encode(nfc(s.value));
    case TypeTag.Integer:
      return utf8.encode(canonicalInteger(s.value));
    case TypeTag.Decimal:
      return utf8.encode(canonicalDecimal(s.value));
    case TypeTag.Bytes:
      return s.value;
    default: {
      const _exhaustive: never = s;
      throw new Error(`unknown tag ${(_exhaustive as {tag: number}).tag}`);
    }
  }
}

/** Canonical string form stored in `data` (self-describing): the value as a string. */
export function asString(s: TypedScalar): string {
  switch (s.tag) {
    case TypeTag.Null:
      return "";
    case TypeTag.Bool:
      return s.value ? "true" : "false";
    case TypeTag.String:
      return nfc(s.value);
    case TypeTag.Integer:
      return canonicalInteger(s.value);
    case TypeTag.Decimal:
      return canonicalDecimal(s.value);
    case TypeTag.Bytes:
      return bytesToHex(s.value);
  }
}

export function bytesToHex(b: Uint8Array): string {
  let h = "";
  for (const x of b) h += x.toString(16).padStart(2, "0");
  return h;
}

export function hexToBytes(h: string): Uint8Array {
  const s = h.startsWith("0x") ? h.slice(2) : h;
  if (s.length % 2 !== 0) throw new Error("odd hex length");
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}
