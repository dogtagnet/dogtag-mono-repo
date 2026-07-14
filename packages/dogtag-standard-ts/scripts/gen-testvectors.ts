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
  verifyInclusion,
  toHex32,
  type ProofStep,
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
  const {root} = buildMerkle(set);
  merkle.push({name: `size_${k}`, leaf_hexes: set.map(toHex32), root_hex: toHex32(root)});
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

// ---- Inclusion proofs (DSDP plan §2.3): the `Sibling | Promote` generate/verify conformance. ----
// The reference vectors M1 rests on. Shared Rust↔TS↔Swift: each verifier RECOMPUTES the leaf from
// (keyPath, salt, tag, value) under DS_LEAF, folds the root-ward steps (sibling→Poseidon3(DS_NODE),
// promote→pass-through), and MUST agree with `valid`. Coverage: every leaf of every tree in the
// audit's leaf counts {1,2,3,5,6,7,13,24,34} (so multi-level promotion — the sorted-last leaf of an
// odd tree promotes at consecutive levels — is exercised end-to-end), a mixed-leaf-type tree (proves
// recompute-from-fields across tags in the inclusion path), and negatives (tampered value, corrupted
// sibling, wrong root) that MUST cleanly return false. This generator self-checks every vector.
const INCLUSION_COUNTS = [1, 2, 3, 5, 6, 7, 13, 24, 34];

interface LeafRecord {
  keyPath: string;
  salt: Uint8Array;
  scalar: TypedScalar;
}

interface InclusionVec {
  name: string;
  leafCount: number;
  keyPath: string;
  saltHex: string;
  tag: number;
  value: string | null;
  steps: ({sibling: string} | {promote: true})[];
  root: string;
  promotes: number; // # of Promote steps (drives the "multi-level promotion exercised" assertion)
  valid: boolean;
}

/** Serialize the in-memory ProofStep[] to the wire shape (plan §4.4). */
function stepsToJson(steps: ProofStep[]): ({sibling: string} | {promote: true})[] {
  return steps.map((s) => ("sibling" in s ? {sibling: toHex32(s.sibling)} : ({promote: true} as const)));
}

/** The packed `value` field as it appears in the vector JSON (mirror of `leafVec`). */
function valueOf(scalar: TypedScalar): string | null {
  if (scalar.tag === TypeTag.Null) return null;
  if (scalar.tag === TypeTag.Bytes) return bytesToHex(scalar.value);
  if (scalar.tag === TypeTag.Bool) return scalar.value ? "true" : "false";
  return scalar.value;
}

function intLeafRecord(i: number): LeafRecord {
  return {keyPath: `leaf${i}`, salt: salt(100 + i), scalar: {tag: TypeTag.Integer, value: String(i)}};
}

const inclusion: InclusionVec[] = [];

function pushValid(name: string, leafCount: number, r: LeafRecord, steps: ProofStep[], root: bigint) {
  if (!verifyInclusion(r.keyPath, r.salt, r.scalar, steps, root)) {
    throw new Error(`gen: valid inclusion vector ${name} failed to verify`);
  }
  inclusion.push({
    name,
    leafCount,
    keyPath: r.keyPath,
    saltHex: bytesToHex(r.salt),
    tag: r.scalar.tag,
    value: valueOf(r.scalar),
    steps: stepsToJson(steps),
    root: toHex32(root),
    promotes: steps.filter((s) => "promote" in s).length,
    valid: true,
  });
}

for (const k of INCLUSION_COUNTS) {
  const records = Array.from({length: k}, (_, i) => intLeafRecord(i));
  const hashes = records.map((r) => hashLeaf(r.keyPath, r.salt, r.scalar));
  const {root, layers} = buildMerkle(hashes);
  const proofs = hashes.map((h) => merkleProof(layers, h));
  records.forEach((r, i) => pushValid(`size_${k}_${r.keyPath}`, k, r, proofs[i]!, root));

  // Negatives for sizes that span promotion depth {5,7,24}: aim them at the leaf with the MOST
  // promote steps so the tampered/corrupted paths also traverse multi-level promotion.
  if (k === 5 || k === 7 || k === 24) {
    let ti = 0;
    for (let i = 1; i < k; i++) {
      if (proofs[i]!.filter((s) => "promote" in s).length > proofs[ti]!.filter((s) => "promote" in s).length) ti = i;
    }
    const r = records[ti]!;
    const steps = proofs[ti]!;

    // (1) tampered value: recomputes a different leaf that will not fold to root.
    const tamperedScalar: TypedScalar = {tag: TypeTag.Integer, value: `${ti}7`};
    if (verifyInclusion(r.keyPath, r.salt, tamperedScalar, steps, root)) throw new Error(`gen: tampered ${k}/${ti} still verified`);
    inclusion.push({
      name: `size_${k}_${r.keyPath}_tampered_value`,
      leafCount: k, keyPath: r.keyPath, saltHex: bytesToHex(r.salt), tag: TypeTag.Integer,
      value: `${ti}7`, steps: stepsToJson(steps), root: toHex32(root),
      promotes: steps.filter((s) => "promote" in s).length, valid: false,
    });

    // (2) corrupted sibling: replace the first real sibling with a DIFFERENT real field element
    // (a valid leaf hash not in this tree) so all three languages reject cleanly (never throw).
    const bogus = hashLeaf("bogus_sibling", salt(240), {tag: TypeTag.Integer, value: "999999"});
    const sibIdx = steps.findIndex((s) => "sibling" in s);
    if (sibIdx >= 0) {
      const corrupted: ProofStep[] = steps.map((s, j) => (j === sibIdx ? {sibling: bogus} : s));
      if (verifyInclusion(r.keyPath, r.salt, r.scalar, corrupted, root)) throw new Error(`gen: corrupted sibling ${k}/${ti} still verified`);
      inclusion.push({
        name: `size_${k}_${r.keyPath}_corrupted_sibling`,
        leafCount: k, keyPath: r.keyPath, saltHex: bytesToHex(r.salt), tag: TypeTag.Integer,
        value: valueOf(r.scalar), steps: stepsToJson(corrupted), root: toHex32(root),
        promotes: corrupted.filter((s) => "promote" in s).length, valid: false,
      });
    }

    // (3) wrong root: correct leaf + steps, but the root of a DIFFERENT tree.
    const otherRoot = buildMerkle(leafSet(k + 1)).root;
    if (verifyInclusion(r.keyPath, r.salt, r.scalar, steps, otherRoot)) throw new Error(`gen: wrong root ${k}/${ti} still verified`);
    inclusion.push({
      name: `size_${k}_${r.keyPath}_wrong_root`,
      leafCount: k, keyPath: r.keyPath, saltHex: bytesToHex(r.salt), tag: TypeTag.Integer,
      value: valueOf(r.scalar), steps: stepsToJson(steps), root: toHex32(otherRoot),
      promotes: steps.filter((s) => "promote" in s).length, valid: false,
    });
  }
}

// Mixed-leaf-type tree (size 6): disclose one leaf of each representative tag through an inclusion
// proof, proving recompute-from-fields works across tags (not just integers) in the fold path.
{
  const mixed: LeafRecord[] = [
    {keyPath: "credentialSubject.dogTagId", salt: salt(150), scalar: {tag: TypeTag.Integer, value: "42"}},
    {keyPath: "credentialSubject.name", salt: salt(151), scalar: {tag: TypeTag.String, value: "Rex"}},
    {keyPath: "flags.lost", salt: salt(152), scalar: {tag: TypeTag.Bool, value: false}},
    {keyPath: "weightHistory[0].value", salt: salt(153), scalar: {tag: TypeTag.Decimal, value: "22.7"}},
    {keyPath: "photoHashes[0]", salt: salt(154), scalar: {tag: TypeTag.Bytes, value: hexToBytes("deadbeef")}},
    {keyPath: "note", salt: salt(155), scalar: {tag: TypeTag.Null, value: null}},
  ];
  const hashes = mixed.map((r) => hashLeaf(r.keyPath, r.salt, r.scalar));
  const {root, layers} = buildMerkle(hashes);
  mixed.forEach((r, i) => pushValid(`mixed6_${r.keyPath}`, mixed.length, r, merkleProof(layers, hashes[i]!), root));
}

if (!inclusion.some((v) => v.valid && v.promotes >= 2)) {
  throw new Error("gen: no inclusion vector exercises multi-level promotion (>= 2 Promote steps)");
}

const out = {
  _comment:
    "Shared DogTag SDK test vectors (impl §9; inclusion proofs per DSDP plan §2.3). TS = reference; " +
    "dogtag-standard-rs + the iOS Swift verifier assert this file. " +
    "Leaf = Poseidon(DS_LEAF, fieldOf(keyPath), fieldOf(salt), fieldOf(typeTag), fieldOf(value)); " +
    "inclusion steps are root-ward {sibling:0x..}|{promote:true}; salts are fixed for reproducibility.",
  field_p: FIELD_P.toString(),
  leaves,
  bytesToField: btf,
  merkle,
  inclusion,
};

const path = resolve(__dirname, "..", "testvectors.json");
writeFileSync(path, JSON.stringify(out, null, 2) + "\n");
console.log(
  `wrote ${path}: ${leaves.length} leaf, ${btf.length} bytesToField, ${merkle.length} merkle, ` +
    `${inclusion.length} inclusion vectors (${inclusion.filter((v) => v.valid).length} valid, ` +
    `${inclusion.filter((v) => !v.valid).length} negative)`,
);
