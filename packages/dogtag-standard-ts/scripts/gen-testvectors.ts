// Generate the shared testvectors.json (impl §9) — inputs -> expected leaf hashes, roots, proofs.
// The TS SDK is the reference; crates/dogtag-standard-rs asserts the SAME file in CI, guaranteeing
// cross-language determinism. Salts are FIXED here so vectors are reproducible.
import {writeFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {
  FIELD_P,
  TypeTag,
  bytesToField,
  buildMerkle,
  hashLeaf,
  merkleProof,
  toHex32,
  type TypedScalar,
} from "../src/index.js";
import {hexToBytes, bytesToHex} from "../src/encode.js";

const __dirname = dirname(fileURLToPath(import.meta.url));

function salt(n: number): Uint8Array {
  // deterministic 16-byte salt: 0x{nn} repeated
  const s = new Uint8Array(16).fill(n & 0xff);
  return s;
}

interface LeafVec {
  name: string;
  keyPath: string;
  saltHex: string;
  tag: number;
  // value encoding: for Bytes (5) -> hex string; else -> string; null -> null
  value: string | null;
  expected_hex: string;
}

function leafVec(name: string, keyPath: string, s: Uint8Array, scalar: TypedScalar): LeafVec {
  const h = hashLeaf(keyPath, s, scalar);
  let value: string | null;
  if (scalar.tag === TypeTag.Null) value = null;
  else if (scalar.tag === TypeTag.Bytes) value = bytesToHex(scalar.value);
  else if (scalar.tag === TypeTag.Bool) value = scalar.value ? "true" : "false";
  else value = scalar.value;
  return {name, keyPath, saltHex: bytesToHex(s), tag: scalar.tag, value, expected_hex: toHex32(h)};
}

const leaves: LeafVec[] = [
  leafVec("null", "a.b", salt(1), {tag: TypeTag.Null, value: null}),
  leafVec("bool_true", "flags.active", salt(2), {tag: TypeTag.Bool, value: true}),
  leafVec("bool_false", "flags.lost", salt(3), {tag: TypeTag.Bool, value: false}),
  leafVec("string_basic", "credentialSubject.name", salt(4), {tag: TypeTag.String, value: "Rex"}),
  // tag 2 "5" must differ from tag 3 5 (mandatory negative — §11.2)
  leafVec("string_five", "x", salt(5), {tag: TypeTag.String, value: "5"}),
  leafVec("integer_five", "x", salt(5), {tag: TypeTag.Integer, value: "5"}),
  // microchip is a 15-digit STRING with leading-zero preservation
  leafVec("microchip", "credentialSubject.microchip.code", salt(6), {
    tag: TypeTag.String,
    value: "985141006580311",
  }),
  leafVec("microchip_leadingzero", "credentialSubject.microchip.code", salt(7), {
    tag: TypeTag.String,
    value: "012345678901234",
  }),
  // decimals from the spec
  leafVec("decimal_weight", "weightHistory[0].value", salt(8), {tag: TypeTag.Decimal, value: "22.7"}),
  leafVec("decimal_titer", "titer.resultIUml", salt(9), {tag: TypeTag.Decimal, value: "0.5"}),
  leafVec("decimal_trailingzeros", "w", salt(10), {tag: TypeTag.Decimal, value: "22.70"}), // == 22.7
  // a timestamp value containing ":" (first-two-colons parse must survive in `data`)
  leafVec("timestamp", "vaccinationDate", salt(11), {tag: TypeTag.String, value: "2026-06-17T14:46:29Z"}),
  // NFC combining sequence normalizes to its composed form
  leafVec("nfc_combining", "note", salt(12), {tag: TypeTag.String, value: "é"}), // -> é
  leafVec("bytes", "photoHashes[0]", salt(13), {tag: TypeTag.Bytes, value: hexToBytes("deadbeef") as never}),
  // a large string spanning multiple 31-byte limbs
  leafVec("long_string", "taskDescription", salt(14), {tag: TypeTag.String, value: "x".repeat(200)}),
];

// bytesToField edge vectors (impl §11.10(d))
const btf = [
  {name: "empty", inputHex: ""},
  {name: "a", inputHex: bytesToHex(new TextEncoder().encode("a"))},
  {name: "a_nul", inputHex: bytesToHex(new TextEncoder().encode("a\x00"))},
  {name: "31_bytes", inputHex: "ab".repeat(31)},
  {name: "32_bytes", inputHex: "cd".repeat(32)},
].map((v) => ({...v, expected_hex: toHex32(bytesToField(hexToBytes(v.inputHex)))}));

// Merkle vectors at sizes 1..9 (odd promotion + single-leaf root), plus a commutativity swap.
function leafSet(k: number): bigint[] {
  const out: bigint[] = [];
  for (let i = 0; i < k; i++) {
    out.push(hashLeaf(`leaf${i}`, salt(100 + i), {tag: TypeTag.Integer, value: String(i)}));
  }
  return out;
}
const merkle = [];
for (let k = 1; k <= 9; k++) {
  const set = leafSet(k);
  const {root, layers} = buildMerkle(set);
  // a proof for the first leaf (in original order)
  const proof = merkleProof(layers, set.slice().sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))[0]!);
  merkle.push({
    name: `size_${k}`,
    leaf_hexes: set.map(toHex32),
    root_hex: toHex32(root),
    first_sorted_leaf_proof: proof.map(toHex32),
  });
}
// commutativity: reversed input order -> same root
{
  const set = leafSet(2);
  const a = buildMerkle(set).root;
  const b = buildMerkle([...set].reverse()).root;
  merkle.push({name: "commutativity_2", leaf_hexes: set.map(toHex32), root_hex: toHex32(a), reversed_root_hex: toHex32(b)});
}
// obfuscation invariance: dropping a cleartext leaf into `obfuscated` keeps the SAME root,
// because the root is over the SAME leaf-hash multiset.
{
  const set = leafSet(5);
  merkle.push({name: "obfuscation_5_same_root", leaf_hexes: set.map(toHex32), root_hex: toHex32(buildMerkle(set).root)});
}

const out = {
  _comment:
    "Shared DogTag SDK test vectors (impl §9). TS = reference; dogtag-standard-rs asserts this file. " +
    "Leaf = Poseidon(DS_LEAF, fieldOf(keyPath), fieldOf(salt), fieldOf(typeTag), fieldOf(value)); " +
    "salts are fixed for reproducibility.",
  field_p: FIELD_P.toString(),
  leaves,
  bytesToField: btf,
  merkle,
};

const path = resolve(__dirname, "..", "testvectors.json");
writeFileSync(path, JSON.stringify(out, null, 2) + "\n");
console.log(`wrote ${path}: ${leaves.length} leaf, ${btf.length} bytesToField, ${merkle.length} merkle vectors`);
