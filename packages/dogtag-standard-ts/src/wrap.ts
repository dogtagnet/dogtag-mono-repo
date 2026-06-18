// Wrap / selective-disclosure / packed-value parsing (impl §1.4, §1.5, §11.2 F2b).
import {buildMerkle} from "./merkle.js";
import {hashLeaf} from "./leaf.js";
import {flatten, unflatten} from "./flatten.js";
import {asString, bytesToHex, hexToBytes} from "./encode.js";
import {toHex32, type Field} from "./field.js";
import {TypeTag, type IssuerMeta, type TypedScalar, type WrappedDoc} from "./types.js";

export type SaltProvider = () => Uint8Array;

function defaultSalt(): Uint8Array {
  const s = new Uint8Array(16);
  // Node 18+/browsers expose Web Crypto globally.
  crypto.getRandomValues(s);
  return s;
}

/** parse(packed): split on the FIRST TWO ":" only (value may contain ":"). impl §11.2 F2b. */
export function parsePacked(packed: string): {saltHex: string; tag: TypeTag; valueRest: string} {
  const first = packed.indexOf(":");
  const second = packed.indexOf(":", first + 1);
  if (first < 0 || second < 0) throw new Error(`bad packed value: ${packed}`);
  const saltHex = packed.slice(0, first);
  const tag = Number(packed.slice(first + 1, second)) as TypeTag;
  const valueRest = packed.slice(second + 1);
  return {saltHex, tag, valueRest};
}

/** Reconstruct a TypedScalar from a packed `tag:valueRest`. */
export function scalarFromPacked(tag: TypeTag, valueRest: string): TypedScalar {
  switch (tag) {
    case TypeTag.Null:
      return {tag, value: null};
    case TypeTag.Bool:
      return {tag, value: valueRest === "true"};
    case TypeTag.String:
      return {tag, value: valueRest};
    case TypeTag.Integer:
      return {tag, value: valueRest};
    case TypeTag.Decimal:
      return {tag, value: valueRest};
    case TypeTag.Bytes:
      return {tag, value: hexToBytes(valueRest)};
    default:
      throw new Error(`unknown tag ${tag}`);
  }
}

/** Recompute the leaf hash for one packed entry (used by verify + obfuscate). */
export function leafFromPacked(keyPath: string, packed: string): Field {
  const {saltHex, tag, valueRest} = parsePacked(packed);
  return hashLeaf(keyPath, hexToBytes(saltHex), scalarFromPacked(tag, valueRest));
}

/** Collect every (keyPath, packed) pair from a nested `data` object. */
export function flattenData(data: unknown): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  const walk = (node: unknown, path: string) => {
    if (typeof node === "string") {
      out.push([path, node]);
      return;
    }
    if (Array.isArray(node)) {
      node.forEach((el, i) => walk(el, `${path}[${i}]`));
      return;
    }
    if (typeof node === "object" && node !== null) {
      for (const k of Object.keys(node)) {
        const childPath = path === "" ? k : `${path}.${k}`;
        walk((node as Record<string, unknown>)[k], childPath);
      }
    }
  };
  walk(data, "");
  return out;
}

/** wrapDocument — typed input -> single Poseidon root R (impl §1.4). */
export function wrapDocument(
  typedCredential: unknown,
  issuer: IssuerMeta,
  saltProvider: SaltProvider = defaultSalt,
): WrappedDoc {
  const flat = flatten(typedCredential);
  const dataFlat: Record<string, string> = {};
  const leaves: Field[] = [];
  for (const {keyPath, scalar} of flat) {
    const salt = saltProvider();
    if (salt.length !== 16) throw new Error("salt provider must yield 16 bytes");
    dataFlat[keyPath] = `${bytesToHex(salt)}:${scalar.tag}:${asString(scalar)}`;
    leaves.push(hashLeaf(keyPath, salt, scalar));
  }
  const {root} = buildMerkle(leaves);
  const R = toHex32(root);
  return {
    version: "dogtag/1.0",
    data: unflatten(dataFlat),
    signature: {type: "DogTagMerkleProof", targetHash: R, proof: [], merkleRoot: R},
    privacy: {obfuscated: []},
    issuer,
  };
}

/** obfuscate — move a field's leaf hash into privacy.obfuscated[] and drop its cleartext. Root unchanged. */
export function obfuscate(doc: WrappedDoc, keyPaths: string[]): WrappedDoc {
  const dataFlat = Object.fromEntries(flattenData(doc.data));
  const obfuscated = [...doc.privacy.obfuscated];
  for (const kp of keyPaths) {
    const packed = dataFlat[kp];
    if (packed === undefined) throw new Error(`cannot obfuscate missing field: ${kp}`);
    obfuscated.push(toHex32(leafFromPacked(kp, packed)));
    delete dataFlat[kp];
  }
  return {...doc, data: unflatten(dataFlat), privacy: {obfuscated}};
}
