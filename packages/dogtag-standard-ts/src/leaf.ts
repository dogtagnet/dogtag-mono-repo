// Poseidon leaf hashing (impl §1.2, §11.2). The leaf is one fixed-arity (5-input / t=6) call:
//   Poseidon(DS_LEAF, fieldOf(keyPath), fieldOf(salt), fieldOf(typeTag), fieldOf(value))
// where keyPath/value use the length-prefixed bytesToField fold, and salt(16B)/typeTag(1B) pack
// directly into one field (§11.2(a)).
import {DS_LEAF, bytesToField, fieldFromScalarBytes, fieldFromUint, poseidon, type Field} from "./field.js";
import {encodeValue, nfc} from "./encode.js";
import {TypeTag, type TypedScalar} from "./types.js";

const utf8 = new TextEncoder();

export function fieldOfKeyPath(keyPath: string): Field {
  return bytesToField(utf8.encode(nfc(keyPath)));
}

export function fieldOfValue(s: TypedScalar): Field {
  return bytesToField(encodeValue(s));
}

/** hashLeaf — Poseidon over the canonical (keyPath, salt, typeTag, value) tuple. */
export function hashLeaf(keyPath: string, salt: Uint8Array, s: TypedScalar): Field {
  if (salt.length !== 16) throw new Error(`salt must be 16 bytes (got ${salt.length})`);
  return poseidon([
    DS_LEAF,
    fieldOfKeyPath(keyPath),
    fieldFromScalarBytes(salt),
    fieldFromUint(s.tag as TypeTag),
    fieldOfValue(s),
  ]);
}
