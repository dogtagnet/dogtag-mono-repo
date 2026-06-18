// Gate A — generate the NORMATIVE poseidon-vectors.json (architecture §13.9(b), impl §11.2(d)/§9).
//
// circomlib is the REFERENCE-OF-RECORD. This script:
//   1. compiles the circomlib Poseidon circuit at every arity the system uses (2,3,5,6 inputs),
//   2. computes the circuit WITNESS for each pinned input vector (= the circom ground truth),
//   3. cross-checks poseidon-lite (TS) and circomlibjs against the circuit output,
//   4. asserts the canonical anchor poseidon([1,2]) == 0x115cc0f5…189a,
//   5. writes poseidon-vectors.json (decimal + 32-byte big-endian hex) for Rust + Solidity to assert.
//
// Any divergence aborts with a non-zero exit (the lockfile/CI gate).
import {execSync} from "node:child_process";
import {existsSync, mkdirSync, readFileSync, writeFileSync, rmSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {poseidon2, poseidon3, poseidon5, poseidon6} from "poseidon-lite";
import {buildPoseidon} from "circomlibjs";
import * as snarkjs from "snarkjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const BUILD = resolve(ROOT, "build", "parity");
const CLIB = resolve(ROOT, "node_modules", "circomlib", "circuits");

const ANCHOR_HEX = "0x115cc0f5e7d690413df64c6b9662e9cf2a3617f2743245519e19607a4417189a";
const ANCHOR_DEC = "7853200120776062878684798364095072458815029376092732009249414926327459813530";

// Domain tags (impl §1.2): DS_LEAF=1, DS_NODE=2, DS_BYTES=3, DS_NULLIFIER=4.
const DS = {LEAF: 1n, NODE: 2n, BYTES: 3n, NULLIFIER: 4n};

// ---- pinned input vectors, grouped by circomlib arity (number of inputs) ----
// Inputs are decimal-string bigints so the JSON is language-neutral.
const VECTORS = [
  // arity 2 / width t=3
  {name: "anchor_1_2", arity: 2, in: [1n, 2n]},
  {name: "fold_bytes_zero", arity: 2, in: [DS.BYTES, 0n]},
  {name: "fold_bytes_sample", arity: 2, in: [DS.BYTES, 1311768467463790320n]},
  // arity 3 / width t=4  (Merkle node: DS_NODE, lo, hi)
  {name: "node_basic", arity: 3, in: [DS.NODE, 111n, 222n]},
  {name: "node_anchor_children", arity: 3, in: [DS.NODE, 1n, 2n]},
  // arity 5 / width t=6  (leaf: DS_LEAF, kp, salt, tag, val)
  {name: "leaf_basic", arity: 5, in: [DS.LEAF, 12345n, 67890n, 2n, 985141006580311n]},
  // arity 6 / width t=7  (nullifier: DS_NULLIFIER, dogTagId, purpose, relayer, subject, nonce)
  {name: "nullifier_basic", arity: 6, in: [DS.NULLIFIER, 42n, 7n, 0x1111111111111111111111111111111111111111n, 0x2222222222222222222222222222222222222222n, 99n]},
];

const ARITY_FN = {2: poseidon2, 3: poseidon3, 5: poseidon5, 6: poseidon6};

function toHex32(x) {
  const h = BigInt(x).toString(16).padStart(64, "0");
  if (h.length > 64) throw new Error("field element exceeds 32 bytes: " + x);
  return "0x" + h;
}

// ---- circom reference-of-record: compile each arity once, compute witnesses ----
function ensureCircomCircuit(arity) {
  mkdirSync(BUILD, {recursive: true});
  const src = resolve(BUILD, `p${arity}.circom`);
  writeFileSync(src,
    `pragma circom 2.1.9;\ninclude "poseidon.circom";\n` +
    `template P(){signal input in[${arity}];signal output out;` +
    `component h=Poseidon(${arity});for(var i=0;i<${arity};i++){h.inputs[i]<==in[i];}out<==h.out;}\n` +
    `component main = P();\n`);
  const wasmDir = resolve(BUILD, `p${arity}_js`);
  if (!existsSync(resolve(wasmDir, `p${arity}.wasm`))) {
    execSync(`circom "${src}" --wasm --output "${BUILD}" -l "${CLIB}"`, {stdio: "pipe"});
  }
  return resolve(wasmDir, `p${arity}.wasm`);
}

async function circomOutput(arity, inputs) {
  const wasm = ensureCircomCircuit(arity);
  const wtnsPath = resolve(BUILD, `w_${arity}.wtns`);
  await snarkjs.wtns.calculate({in: inputs.map(String)}, wasm, wtnsPath);
  const w = await snarkjs.wtns.exportJson(wtnsPath);
  // witness[0] = 1 (constant), witness[1] = main.out (single output signal)
  return BigInt(w[1]);
}

async function main() {
  const poseidonCjs = await buildPoseidon();
  const Fcjs = poseidonCjs.F;
  const FIELD_R = Fcjs.p; // BN254 scalar field r (ground truth modulus from circomlibjs)

  const out = {
    _comment: "NORMATIVE Poseidon parity vectors (Gate A). circomlib = reference-of-record. " +
      "Asserted bit-identical across circom (this file's generator) + poseidon-lite (TS) + " +
      "light-poseidon new_circom (Rust) + poseidon-solidity PoseidonT3..T7 (Solidity).",
    field_r: FIELD_R.toString(),
    anchor: {dec: ANCHOR_DEC, hex: ANCHOR_HEX},
    domain_tags: {DS_LEAF: 1, DS_NODE: 2, DS_BYTES: 3, DS_NULLIFIER: 4},
    vector_count: VECTORS.length,
    vectors: [],
  };

  let anchorSeen = false;
  for (const v of VECTORS) {
    const fn = ARITY_FN[v.arity];
    if (!fn) throw new Error("no poseidon-lite fn for arity " + v.arity);

    const lite = fn(v.in);                                  // TS candidate
    const cjs = Fcjs.toObject(poseidonCjs(v.in));           // circomlibjs (pinned + anchor-tested)
    const circom = await circomOutput(v.arity, v.in);       // reference-of-record

    // all three MUST agree with the circom witness
    if (lite !== circom) throw new Error(`poseidon-lite != circom @ ${v.name}: ${lite} vs ${circom}`);
    if (cjs !== circom) throw new Error(`circomlibjs != circom @ ${v.name}: ${cjs} vs ${circom}`);

    if (v.name === "anchor_1_2") {
      anchorSeen = true;
      if (circom !== BigInt(ANCHOR_DEC)) throw new Error(`ANCHOR MISMATCH: ${circom} != ${ANCHOR_DEC}`);
    }

    out.vectors.push({
      name: v.name,
      arity: v.arity,
      width: v.arity + 1,
      in: v.in.map(String),
      out_dec: circom.toString(),
      out_hex: toHex32(circom),
    });
    console.log(`  ✓ ${v.name.padEnd(22)} arity=${v.arity} (t=${v.arity + 1})  circom==lite==circomlibjs  ${toHex32(circom)}`);
  }
  if (!anchorSeen) throw new Error("anchor vector poseidon([1,2]) missing");

  writeFileSync(resolve(ROOT, "poseidon-vectors.json"), JSON.stringify(out, null, 2) + "\n");
  console.log(`\nGate A (circom + TS) GREEN — wrote poseidon-vectors.json (${out.vectors.length} vectors, field_r pinned).`);
  // free any worker threads snarkjs/ffjavascript may hold open
  try { rmSync(resolve(BUILD, "w_2.wtns"), {force: true}); } catch {}
  process.exit(0);
}

main().catch((e) => { console.error("Gate A FAILED:", e.message); process.exit(1); });
