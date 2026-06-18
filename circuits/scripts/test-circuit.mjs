// DogTag verification circuit test harness (impl §11.9(d), §9 root-parity gate).
//
// Validates:
//   (1) witness/proof ROUND-TRIP: build a real N=8 credential, EdDSA-sign the consent
//       message, calculate the witness, groth16 prove + verify, and assert the public
//       signals == independently-computed [dogTagId, purpose, relayer, subject, nullifier,
//       keyHash, R].
//   (2) R-PARITY (§9): the circuit's output R == the SDK buildMerkle root, bit-for-bit.
//   (3) NEGATIVE tests: tampered dogTagId value, bad EdDSA signature, tampered nullifier
//       input all fail.
import {execFileSync} from "node:child_process";
import {existsSync, mkdtempSync, writeFileSync, readFileSync, rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {poseidon2, poseidon3, poseidon5, poseidon6} from "poseidon-lite";
import {buildEddsa} from "circomlibjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const BUILD = resolve(ROOT, "build");
const WASM = resolve(BUILD, "verification_js", "verification.wasm");
const WGEN = resolve(BUILD, "verification_js", "generate_witness.js");
const ZKEY = resolve(BUILD, "verification_final.zkey");
const VKEY = resolve(BUILD, "verification_key.json");

// --- SDK ground truth (import the BUILT SDK dist directly: same poseidon-lite, same rules) ---
const SDK = resolve(ROOT, "..", "packages", "dogtag-standard-ts", "dist");
const {buildMerkle} = await import(resolve(SDK, "merkle.js"));
const {DS_LEAF, DS_NODE, DS_NULLIFIER, FIELD_P} = await import(resolve(SDK, "field.js"));

const N = 24; // PRODUCTION: max leaves
const SNARKJS = resolve(ROOT, "node_modules", ".bin", "snarkjs");
const snarkjsBin = existsSync(SNARKJS)
  ? SNARKJS
  : resolve(ROOT, "..", "node_modules", ".bin", "snarkjs");

function sh(args, cwd = ROOT) {
  return execFileSync(snarkjsBin, args, {cwd, stdio: ["ignore", "pipe", "pipe"]}).toString();
}

// poseidon-lite dispatched by arity (mirrors field.ts poseidon()).
function poseidon(inputs) {
  switch (inputs.length) {
    case 2: return poseidon2(inputs);
    case 3: return poseidon3(inputs);
    case 5: return poseidon5(inputs);
    case 6: return poseidon6(inputs);
    default: throw new Error("unsupported arity " + inputs.length);
  }
}

// hashLeaf in the in-circuit (already-reduced) form: Poseidon5([DS_LEAF, kp, salt, tag, val]).
function hashLeaf(kp, salt, tag, val) {
  return poseidon([DS_LEAF, kp, salt, tag, val]);
}

let passed = 0;
let failed = 0;
function check(name, cond) {
  if (cond) { passed++; console.log(`  ✓ ${name}`); }
  else { failed++; console.error(`  ✗ ${name}`); }
}

// ---------------------------------------------------------------------------
// Build a sample credential witness object for a given numLeaves (1..N=24).
// The first `numLeaves` slots are REAL leaves (one of them the dogTagId leaf);
// slots [numLeaves, N) are inert padding pinned to the inactive diagonal
// (perm[k]==k, sortedLeafHashes[k]==leafHash[k]) so the circuit ignores them.
// The SDK buildMerkle root is computed over EXACTLY the first `numLeaves` leaves.
// ---------------------------------------------------------------------------
async function buildWitness(eddsa, opts = {}) {
  const F = eddsa.babyJub.F;

  const numLeaves = opts.numLeaves ?? 8;
  if (numLeaves < 1 || numLeaves > N) throw new Error("numLeaves out of range");

  // public inputs
  const dogTagId = opts.dogTagId ?? 424242n;
  const purpose = 7n;                 // e.g. GROOMING_INTAKE label reduced mod p
  const relayer = 0x1111111111111111111111111111111111111111n;
  const subject = 0x2222222222222222222222222222222222222222n;
  const consentNonce = 99n;

  // keyPath field for credentialSubject.dogTagId (any fixed constant; bound in-circuit).
  const dogTagKeyPathField = 0xabcdef1234567890n;

  // The dogTagId leaf lives at a canonical index strictly inside the active prefix.
  const dogTagIdLeafIndex = Math.min(3, numLeaves - 1);

  // Build full N-wide canonical arrays. Indices [0,numLeaves) are real; the rest are
  // padding (kept distinct so leafHash values stay distinct, but never folded).
  const leafKeyPathHashes = [];
  const leafSalts = [];
  const leafTypeTags = [];
  const leafValues = [];
  for (let i = 0; i < N; i++) {
    if (i === dogTagIdLeafIndex) {
      leafKeyPathHashes.push(dogTagKeyPathField);
      leafSalts.push(1000n + BigInt(i));
      leafTypeTags.push(2n);
      leafValues.push(opts.tamperDogTagValue ? dogTagId + 1n : dogTagId);
    } else {
      leafKeyPathHashes.push(BigInt(100 + i) * 7919n);
      leafSalts.push(1000n + BigInt(i));
      leafTypeTags.push(BigInt((i % 4) + 1));
      leafValues.push(BigInt(500000 + i * 13));
    }
  }

  const leafHashes = leafValues.map((_, i) =>
    hashLeaf(leafKeyPathHashes[i], leafSalts[i], leafTypeTags[i], leafValues[i]));

  // SDK ground-truth root over EXACTLY the active prefix [0, numLeaves).
  const activeHashes = leafHashes.slice(0, numLeaves);
  const sdk = buildMerkle(activeHashes);
  const sortedActive = sdk.layers[0]; // ascending-sorted active leaves
  const R = sdk.root;

  // perm for active prefix: perm[k] = canonical index i s.t. sortedActive[k] == leafHashes[i].
  // Padding slots [numLeaves, N) are pinned to the diagonal (perm[k]==k) and
  // sortedLeafHashes[k] == leafHash[k] (the padding leaf's own hash) — circuit ignores them.
  const sortedLeafHashes = [];
  const perm = [];
  for (let k = 0; k < N; k++) {
    if (k < numLeaves) {
      sortedLeafHashes.push(sortedActive[k]);
      const idx = activeHashes.findIndex((x) => x === sortedActive[k]);
      if (idx < 0) throw new Error("perm: active leaf not found");
      perm.push(idx);
    } else {
      sortedLeafHashes.push(leafHashes[k]); // diagonal: out[k] == in[k]
      perm.push(k);
    }
  }

  // consent message M = Poseidon6([dogTagId, purpose, relayer, subject, R, nonce]) (no DS tag).
  const M = poseidon([dogTagId, purpose, relayer, subject, R, consentNonce]);

  // BabyJubjub EdDSA-Poseidon key + signature.
  const prv = Buffer.from(opts.prvHex ??
    "0001020304050607080900010203040506070809000102030405060708090001", "hex");
  const pub = eddsa.prv2pub(prv);
  const Ax = F.toObject(pub[0]);
  const Ay = F.toObject(pub[1]);
  const sig = eddsa.signPoseidon(prv, F.e(M));
  let R8x = F.toObject(sig.R8[0]);
  let R8y = F.toObject(sig.R8[1]);
  let S = sig.S;
  if (opts.tamperSig) { S = (S + 1n) % FIELD_P; }

  // Expected public outputs.
  const nullifier = poseidon([DS_NULLIFIER, dogTagId, purpose, relayer, subject, consentNonce]);
  const keyHash = poseidon([Ax, Ay]);

  const input = {
    dogTagId: dogTagId.toString(),
    purpose: purpose.toString(),
    relayer: relayer.toString(),
    subject: subject.toString(),
    numLeaves: String(numLeaves),
    leafKeyPathHashes: leafKeyPathHashes.map(String),
    leafTypeTags: leafTypeTags.map(String),
    leafSalts: leafSalts.map(String),
    leafValues: leafValues.map(String),
    dogTagIdLeafIndex: String(dogTagIdLeafIndex),
    sortedLeafHashes: sortedLeafHashes.map(String),
    perm: perm.map(String),
    dogTagKeyPathField: dogTagKeyPathField.toString(),
    consentNonce: consentNonce.toString(),
    Ax: Ax.toString(),
    Ay: Ay.toString(),
    R8x: R8x.toString(),
    R8y: R8y.toString(),
    S: S.toString(),
  };

  return {input, expected: {dogTagId, purpose, relayer, subject, nullifier, keyHash, R, numLeaves}};
}

// ---------------------------------------------------------------------------
// witness calc + prove + verify; returns {ok, publicSignals}.
// ---------------------------------------------------------------------------
async function proveAndVerify(input, tag, mutatePublic) {
  const dir = mkdtempSync(resolve(tmpdir(), "dogtag-" + tag + "-"));
  const inJson = resolve(dir, "input.json");
  const wtns = resolve(dir, "witness.wtns");
  const proof = resolve(dir, "proof.json");
  const pub = resolve(dir, "public.json");
  try {
    writeFileSync(inJson, JSON.stringify(input));
    // witness calculation (throws if constraints unsatisfiable — the negative-test gate).
    sh(["wtns", "calculate", WASM, inJson, wtns], dir);
    sh(["groth16", "prove", ZKEY, wtns, proof, pub], dir);
    let publicSignals = JSON.parse(readFileSync(pub, "utf8"));
    if (mutatePublic) {
      publicSignals = mutatePublic(publicSignals);
      writeFileSync(pub, JSON.stringify(publicSignals));
    }
    // groth16 verify: snarkjs exits non-zero AND prints "Invalid proof" on failure.
    let out;
    try {
      out = sh(["groth16", "verify", VKEY, pub, proof], dir);
    } catch (e) {
      out = (e.stdout?.toString() ?? "") + (e.stderr?.toString() ?? "");
    }
    const ok = /OK/.test(out) && !/Invalid proof/.test(out);
    return {ok, publicSignals};
  } finally {
    rmSync(dir, {recursive: true, force: true});
  }
}

async function main() {
  for (const f of [WASM, ZKEY, VKEY]) {
    if (!existsSync(f)) {
      console.error(`MISSING build artifact: ${f}\nRun: npm run build-circuit (scripts/setup.sh)`);
      process.exit(1);
    }
  }
  const eddsa = await buildEddsa();

  // ================= (1) ROUND-TRIP =================
  console.log("\n[1] witness/proof round-trip + public-signal assertion");
  const {input, expected} = await buildWitness(eddsa);
  const {ok, publicSignals} = await proveAndVerify(input, "roundtrip");
  check("groth16 verify OK", ok);

  // snarkjs public signal vector = [dogTagId, purpose, relayer, subject, nullifier, keyHash, R].
  const want = [
    expected.dogTagId, expected.purpose, expected.relayer, expected.subject,
    expected.nullifier, expected.keyHash, expected.R,
  ].map(String);
  check("public-signal count == 7", publicSignals.length === 7);
  const labels = ["dogTagId", "purpose", "relayer", "subject", "nullifier", "keyHash", "R"];
  for (let i = 0; i < 7; i++) {
    check(`public[${i}] (${labels[i]}) == expected`, publicSignals[i] === want[i]);
  }

  // ================= (2) R-PARITY (§9) =================
  console.log("\n[2] R-PARITY: circuit R == SDK buildMerkle root (bit-for-bit)");
  const circuitR = publicSignals[6];
  check("circuit R == SDK R (numLeaves=8)", circuitR === expected.R.toString());
  console.log(`      SDK R     = ${expected.R}`);
  console.log(`      circuit R = ${circuitR}`);

  // R-PARITY across MANY leaf counts, including ODD (non-power-of-2) counts that exercise
  // odd-promotion at multiple levels. For each, the circuit's output R must equal the SDK
  // buildMerkle root over exactly `numLeaves` leaves.
  console.log("\n[2b] R-PARITY across leaf counts {1,2,3,5,6,7,13,24} (odd-promotion)");
  for (const nl of [1, 2, 3, 5, 6, 7, 13, 24]) {
    const {input: inp, expected: exp} = await buildWitness(eddsa, {numLeaves: nl});
    const {ok: pok, publicSignals: ps} = await proveAndVerify(inp, "parity-" + nl);
    const rOk = ps[6] === exp.R.toString();
    check(`numLeaves=${nl}: prove+verify OK`, pok);
    check(`numLeaves=${nl}: circuit R == SDK buildMerkle root`, rOk);
    if (nl === 1) {
      // Single leaf: SDK root == that leaf; circuit must pass it through unchanged.
      check(`numLeaves=1: R == the single sorted leaf (pass-through)`,
        ps[6] === inp.sortedLeafHashes[0]);
    }
    if (!rOk) {
      console.error(`      numLeaves=${nl} SDK R     = ${exp.R}`);
      console.error(`      numLeaves=${nl} circuit R = ${ps[6]}`);
    }
  }

  // ================= (3) NEGATIVE TESTS =================
  console.log("\n[3] negative tests");

  // 3a: tampered dogTagId leaf value (value != public dogTagId) -> witness gen fails.
  {
    const {input: bad} = await buildWitness(eddsa, {tamperDogTagValue: true});
    let threw = false;
    try { await proveAndVerify(bad, "tamper-value"); } catch { threw = true; }
    check("tampered dogTagId leaf value FAILS witness/proof", threw);
  }

  // 3b: bad EdDSA signature -> witness gen fails (EdDSAPoseidonVerifier ForceEqual).
  {
    const {input: bad} = await buildWitness(eddsa, {tamperSig: true});
    let threw = false;
    try { await proveAndVerify(bad, "bad-sig"); } catch { threw = true; }
    check("bad EdDSA signature FAILS witness/proof", threw);
  }

  // 3c: tampered nullifier PUBLIC SIGNAL -> verify rejects (proof bound to real nullifier).
  {
    const {input: good} = await buildWitness(eddsa);
    const {ok: vok} = await proveAndVerify(good, "tamper-nf", (ps) => {
      const m = [...ps];
      m[4] = (BigInt(m[4]) + 1n).toString(); // nullifier slot
      return m;
    });
    check("tampered nullifier public signal FAILS verify", vok === false);
  }

  console.log(`\n${failed === 0 ? "ALL GREEN" : "FAILURES"} — passed=${passed} failed=${failed}`);
  process.exit(failed === 0 ? 0 : 1);
}

main().catch((e) => { console.error("test-circuit FAILED:", e); process.exit(1); });
