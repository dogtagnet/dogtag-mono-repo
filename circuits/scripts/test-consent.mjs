// DogTagConsent (Level-B, M2) circuit test harness.
//
// Validates the owner-unlinkable consent circuit end-to-end against the SDK ground truth:
//   [1] witness/proof ROUND-TRIP + public-signal assertion in the EXACT M4 order
//       [dogTagId, purpose, relayer, nullifier, R, recordType, deadline].
//   [2] R-PARITY: circuit R == SDK buildMerkle root, across several tree sizes (odd-promotion).
//   [3] keyPath-CONSTANT parity: the circuit's hardcoded fieldOf(keyPath) literals == the SDK's.
//   [4] NEGATIVE tests:
//        (a) wrong signer (tampered EdDSA S)                       -> witness gen fails
//        (b) tampered leaf (ownerSecret != in-tree value)          -> fold misses R, fails
//        (c) inconsistent paths (secret leaf folds to a 2nd root)  -> incSecret.root===R fails
//        (d) tampered R public signal (post-proof)                 -> groth16 verify rejects
//        (e) keyPath SUBSTITUTION (point owner-secret proof at an  -> pinned keyPath blocks it
//            attribute leaf, ownerSecret = that leaf's value)         => witness gen fails  ★
//   [5] NULLIFIER semantics (D5): same nonce -> same nullifier; different nonce -> different.
//
// ★ = the load-bearing security property unique to Level-B (see consent.circom reserved-leaf note).
import {execFileSync} from "node:child_process";
import {existsSync, mkdtempSync, writeFileSync, readFileSync, rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {buildEddsa} from "circomlibjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const BUILD = resolve(ROOT, "build");
const WASM = resolve(BUILD, "consent_js", "consent.wasm");
const ZKEY = resolve(BUILD, "consent_final.zkey");
const VKEY = resolve(BUILD, "consent_verification_key.json");

// --- SDK ground truth (import the BUILT SDK dist: same poseidon-lite, same node rule) ---
const SDK = resolve(ROOT, "..", "packages", "dogtag-standard-ts", "dist");
const {buildMerkle, merkleProof, processProof} = await import(resolve(SDK, "merkle.js"));
const {DS_LEAF, DS_NULLIFIER, FIELD_P, poseidon, fieldFromScalarBytes} =
  await import(resolve(SDK, "field.js"));
const {fieldOfKeyPath} = await import(resolve(SDK, "leaf.js"));

const DEPTH = 6; // must match `component main = DogTagConsent(6)`

// ---- Reserved-leaf schema constants — MUST equal the literals pinned in consent.circom. ----
const KP_OWNER_ADDRESS = 20593649144631820416234157596070441856608371338897391424937040814759273231214n;
const KP_CONSENT_KEY   = 7822071287675030884271946396254564996644565056920260282559292033992393086992n;
const KP_OWNER_SECRET  = 11172449362271989869407103131203633198993612309996015027844083581837121079156n;
const TAG_RESERVED = 5n; // TypeTag.Bytes

const SNARKJS = resolve(ROOT, "node_modules", ".bin", "snarkjs");
const snarkjsBin = existsSync(SNARKJS)
  ? SNARKJS
  : resolve(ROOT, "..", "node_modules", ".bin", "snarkjs");

function sh(args, cwd = ROOT) {
  return execFileSync(snarkjsBin, args, {cwd, stdio: ["ignore", "pipe", "pipe"]}).toString();
}

// leaf hash in the in-circuit (already-reduced) form: Poseidon5([DS_LEAF, kp, salt, tag, val]).
function hashLeaf5(kp, saltField, tag, valField) {
  return poseidon([DS_LEAF, kp, saltField, tag, valField]);
}

// 16-byte deterministic salt -> field (fieldFromScalarBytes, as leaf.ts packs the 16B salt).
function saltField(seed) {
  const b = new Uint8Array(16);
  for (let i = 0; i < 16; i++) b[i] = (seed * 31 + i * 7 + 1) & 0xff;
  return fieldFromScalarBytes(b);
}

// Adapt the SDK's explicit `Sibling | Promote` inclusion proof (DSDP §2.3) to the circuit's
// FRONT-PACKED sibling array + length. `merkleProof` returns EITHER the current ProofStep form
// [{sibling},…,{promote:true},…] OR — from an older built SDK dist — a bare sibling-field array
// (promotes omitted). The circuit (MerkleInclusion) folds only the real siblings (a {promote} level
// is a pass-through, no hashing — exactly what processProof does), so accept both shapes: keep the
// real siblings, drop any {promote} pass-through level, set pathLen to that count, pad the tail with 0s.
function padPath(proof) {
  const siblings = proof
    .filter((s) => typeof s !== "object" || "sibling" in s)
    .map((s) => (typeof s === "object" ? s.sibling : s).toString());
  if (siblings.length > DEPTH) throw new Error(`path too deep (${siblings.length} > ${DEPTH})`);
  const pathLen = siblings.length;
  while (siblings.length < DEPTH) siblings.push("0");
  return {siblings, pathLen: String(pathLen)};
}

let passed = 0;
let failed = 0;
function check(name, cond) {
  if (cond) { passed++; console.log(`  ✓ ${name}`); }
  else { failed++; console.error(`  ✗ ${name}`); }
}

// ---------------------------------------------------------------------------
// Build a Level-B witness: 3 reserved owner leaves + `numAttrs` disclosable attribute leaves,
// folded into one per-tag tree with root R. EdDSA-sign the consent M, assemble circuit input.
// ---------------------------------------------------------------------------
async function buildConsentWitness(eddsa, opts = {}) {
  const F = eddsa.babyJub.F;

  // ---- public consent context ----
  const dogTagId = opts.dogTagId ?? 424242n;
  const purpose = opts.purpose ?? 7n;
  const relayer = opts.relayer ?? 0x1111111111111111111111111111111111111111n;
  const recordType = opts.recordType ?? 0x0aa0n;
  const deadline = opts.deadline ?? 1893456000n; // 2030-01-01
  const consentNonce = opts.consentNonce ?? 99n;

  // ---- owner private material ----
  const ownerAddress = opts.ownerAddress ?? 0x00000000000000000000000000000000dEaDbeeFn;
  const ownerSecret = opts.ownerSecret ?? 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdefn % FIELD_P;
  const ownerSalt = saltField(11);
  const keySalt = saltField(22);
  const secretSalt = saltField(33);

  // ---- BabyJubJub consent key + EdDSA signature over M ----
  const prv = Buffer.from(opts.prvHex ??
    "0001020304050607080900010203040506070809000102030405060708090001", "hex");
  const pub = eddsa.prv2pub(prv);
  const Ax = F.toObject(pub[0]);
  const Ay = F.toObject(pub[1]);
  const keyHash = poseidon([Ax, Ay]);

  const M = poseidon([dogTagId, purpose, relayer, deadline, consentNonce]);
  // "wrong signer": a DIFFERENT key signs M while the tree/circuit still carry (Ax,Ay) of `prv`.
  const signingPrv = opts.wrongSignerPrvHex
    ? Buffer.from(opts.wrongSignerPrvHex, "hex")
    : prv;
  const sig = eddsa.signPoseidon(signingPrv, F.e(M));
  const R8x = F.toObject(sig.R8[0]);
  const R8y = F.toObject(sig.R8[1]);
  let S = sig.S;
  if (opts.tamperSig) S = (S + 1n) % FIELD_P;

  // ---- the three reserved owner leaves ----
  const ownerLeaf = hashLeaf5(KP_OWNER_ADDRESS, ownerSalt, TAG_RESERVED, ownerAddress);
  const keyLeaf = hashLeaf5(KP_CONSENT_KEY, keySalt, TAG_RESERVED, keyHash);
  const secretLeaf = hashLeaf5(KP_OWNER_SECRET, secretSalt, TAG_RESERVED, ownerSecret);

  // ---- disclosable attribute leaves (arbitrary but distinct) ----
  const numAttrs = opts.numAttrs ?? 5;
  const attrs = []; // {leaf, salt, tag, value, kp}
  for (let i = 0; i < numAttrs; i++) {
    const kp = fieldOfKeyPath(`credentialSubject.attr${i}`);
    const salt = saltField(100 + i);
    const tag = BigInt((i % 4) + 1);
    const value = BigInt(700000 + i * 101);
    attrs.push({leaf: hashLeaf5(kp, salt, tag, value), salt, tag, value, kp});
  }

  const allLeaves = [ownerLeaf, keyLeaf, secretLeaf, ...attrs.map((a) => a.leaf)];
  const tree = buildMerkle(allLeaves);
  const R = tree.root;

  // ---- inclusion paths (front-packed sibling sets, SDK merkleProof) ----
  const ownerPath = padPath(merkleProof(tree.layers, ownerLeaf));
  const keyPath = padPath(merkleProof(tree.layers, keyLeaf));
  const secretPath = padPath(merkleProof(tree.layers, secretLeaf));

  // sanity: SDK processProof must reproduce R from each leaf (pre-circuit gate).
  if (processProof(merkleProof(tree.layers, secretLeaf), secretLeaf) !== R)
    throw new Error("SDK processProof != R (harness bug)");

  const nullifier = poseidon([DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce]);

  const input = {
    dogTagId: dogTagId.toString(),
    purpose: purpose.toString(),
    relayer: relayer.toString(),
    recordType: recordType.toString(),
    deadline: deadline.toString(),
    consentNonce: consentNonce.toString(),
    ownerAddress: ownerAddress.toString(),
    ownerSecret: ownerSecret.toString(),
    Ax: Ax.toString(),
    Ay: Ay.toString(),
    R8x: R8x.toString(),
    R8y: R8y.toString(),
    S: S.toString(),
    ownerSalt: ownerSalt.toString(),
    keySalt: keySalt.toString(),
    secretSalt: secretSalt.toString(),
    ownerSiblings: ownerPath.siblings,
    ownerPathLen: ownerPath.pathLen,
    keySiblings: keyPath.siblings,
    keyPathLen: keyPath.pathLen,
    secretSiblings: secretPath.siblings,
    secretPathLen: secretPath.pathLen,
  };

  // Mutators for negative tests (applied after the honest assembly above).
  if (opts.tamperSecretValue) {
    // ownerSecret no longer matches the in-tree owner-secret leaf -> fold misses R.
    input.ownerSecret = ((ownerSecret + 1n) % FIELD_P).toString();
  }
  if (opts.corruptSecretSibling) {
    // flips one real sibling -> secret leaf folds to a different root than owner/key leaves.
    const sib0 = BigInt(input.secretSiblings[0]);
    input.secretSiblings[0] = ((sib0 + 1n) % FIELD_P).toString();
  }
  if (opts.substituteAttrAsSecret) {
    // ATTACK the pinning: reuse a disclosable attribute leaf as the "owner-secret" leaf.
    // Prover sets ownerSecret := attr.value and feeds the attribute's real salt + inclusion
    // proof. Because the circuit PINS keyPath=KP_OWNER_SECRET & tag=5, the recomputed leaf
    // != the attribute leaf, so the attribute's proof folds to R from the WRONG leaf -> fail.
    const a = attrs[0];
    input.ownerSecret = a.value.toString();
    input.secretSalt = a.salt.toString();
    const ap = padPath(merkleProof(tree.layers, a.leaf));
    input.secretSiblings = ap.siblings;
    input.secretPathLen = ap.pathLen;
  }

  return {
    input,
    expected: {dogTagId, purpose, relayer, recordType, deadline, nullifier, R},
  };
}

async function proveAndVerify(input, tag, mutatePublic) {
  const dir = mkdtempSync(resolve(tmpdir(), "dogtag-consent-" + tag + "-"));
  const inJson = resolve(dir, "input.json");
  const wtns = resolve(dir, "witness.wtns");
  const proof = resolve(dir, "proof.json");
  const pub = resolve(dir, "public.json");
  try {
    writeFileSync(inJson, JSON.stringify(input));
    sh(["wtns", "calculate", WASM, inJson, wtns], dir); // throws on unsatisfiable constraints
    sh(["groth16", "prove", ZKEY, wtns, proof, pub], dir);
    let publicSignals = JSON.parse(readFileSync(pub, "utf8"));
    if (mutatePublic) {
      publicSignals = mutatePublic(publicSignals);
      writeFileSync(pub, JSON.stringify(publicSignals));
    }
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

async function expectWitnessFailure(eddsa, opts, tag) {
  const {input} = await buildConsentWitness(eddsa, opts);
  let threw = false;
  try { await proveAndVerify(input, tag); } catch { threw = true; }
  return threw;
}

async function main() {
  // Heavy manual gate: the consent zkey/wasm/VK are the committed M3 production ceremony output. If
  // they are somehow absent, SKIP cleanly (exit 0) rather than red an unbuilt CI run - regenerate with
  // scripts/ceremony-consent.sh (NOT `npm run build-consent`, the DEV/throwaway setup that would
  // clobber the committed production key).
  for (const f of [WASM, ZKEY, VKEY]) {
    if (!existsSync(f)) {
      console.log(`SKIP test-consent: missing committed artifact ${f}\n  It is the committed M3 production ceremony output; regenerate with scripts/ceremony-consent.sh (NOT build-consent, the DEV/throwaway setup that would clobber the committed production key).`);
      process.exit(0);
    }
  }
  const eddsa = await buildEddsa();

  // ================= [3] keyPath-CONSTANT parity =================
  console.log("\n[3] keyPath-constant parity: circuit literals == SDK fieldOfKeyPath");
  check("KP_OWNER_ADDRESS literal == fieldOfKeyPath('owner.address')",
    fieldOfKeyPath("owner.address") === KP_OWNER_ADDRESS);
  check("KP_CONSENT_KEY literal == fieldOfKeyPath('owner.consentKey')",
    fieldOfKeyPath("owner.consentKey") === KP_CONSENT_KEY);
  check("KP_OWNER_SECRET literal == fieldOfKeyPath('owner.secret')",
    fieldOfKeyPath("owner.secret") === KP_OWNER_SECRET);

  // ================= [1] ROUND-TRIP + public-signal order =================
  console.log("\n[1] witness/proof round-trip + public-signal assertion");
  const {input, expected} = await buildConsentWitness(eddsa);
  const {ok, publicSignals} = await proveAndVerify(input, "roundtrip");
  check("groth16 verify OK", ok);

  // public vector = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline].
  const want = [
    expected.dogTagId, expected.purpose, expected.relayer,
    expected.nullifier, expected.R, expected.recordType, expected.deadline,
  ].map(String);
  check("public-signal count == 7", publicSignals.length === 7);
  const labels = ["dogTagId", "purpose", "relayer", "nullifier", "R", "recordType", "deadline"];
  for (let i = 0; i < 7; i++) {
    check(`public[${i}] (${labels[i]}) == expected`, publicSignals[i] === want[i]);
  }

  // ================= [2] R-PARITY across tree sizes (odd-promotion) =================
  console.log("\n[2] R-PARITY: circuit R == SDK buildMerkle root, tree sizes {3,4,5,7,10,20 leaves}");
  // total leaves = 3 reserved + numAttrs; exercise several counts incl. odd-promotion.
  for (const numAttrs of [0, 1, 2, 4, 7, 17]) {
    const total = 3 + numAttrs;
    const {input: inp, expected: exp} = await buildConsentWitness(eddsa, {numAttrs});
    const {ok: pok, publicSignals: ps} = await proveAndVerify(inp, "parity-" + total);
    check(`${total} leaves: prove+verify OK`, pok);
    check(`${total} leaves: circuit R == SDK buildMerkle root`, ps[4] === exp.R.toString());
  }

  // ================= [4] NEGATIVE tests =================
  console.log("\n[4] negative tests");

  check("(a) tampered EdDSA S FAILS witness/proof",
    await expectWitnessFailure(eddsa, {tamperSig: true}, "bad-sig"));

  check("(a2) wrong signer (different key signs M, tree key unchanged) FAILS witness/proof",
    await expectWitnessFailure(eddsa,
      {wrongSignerPrvHex: "0102030405060708090001020304050607080900010203040506070809000102"}, "wrong-signer"));

  check("(b) tampered owner-secret value (not in tree) FAILS witness/proof",
    await expectWitnessFailure(eddsa, {tamperSecretValue: true}, "tamper-secret"));

  check("(c) inconsistent path (secret leaf folds to a 2nd root) FAILS witness/proof",
    await expectWitnessFailure(eddsa, {corruptSecretSibling: true}, "bad-path"));

  // (d) tampered R public signal after proving -> verify must reject.
  {
    const {input: good} = await buildConsentWitness(eddsa);
    const {ok: vok} = await proveAndVerify(good, "tamper-R", (ps) => {
      const m = [...ps];
      m[4] = (BigInt(m[4]) + 1n).toString(); // R slot
      return m;
    });
    check("(d) tampered R public signal FAILS verify", vok === false);
  }

  // (e) ★ keyPath substitution: attribute leaf reused as owner-secret -> pinned keyPath blocks it.
  check("(e) ★ keyPath substitution (attr leaf as owner-secret) FAILS witness/proof",
    await expectWitnessFailure(eddsa, {substituteAttrAsSecret: true}, "kp-substitute"));

  // ================= [5] NULLIFIER semantics (D5) =================
  console.log("\n[5] nullifier semantics (D5): nonce binds the nullifier");
  {
    const {expected: e1} = await buildConsentWitness(eddsa, {consentNonce: 99n});
    const {input: i1b, expected: e1b} = await buildConsentWitness(eddsa, {consentNonce: 99n});
    const {expected: e2} = await buildConsentWitness(eddsa, {consentNonce: 100n});
    check("same (context,nonce) -> identical nullifier (replay => same NF, rejected on-chain)",
      e1.nullifier === e1b.nullifier);
    check("different nonce -> different nullifier (fresh consent => new NF, allowed)",
      e1.nullifier !== e2.nullifier);
    // and the circuit actually emits that nullifier for the fresh nonce.
    const {ok: ok2, publicSignals: ps2} = await proveAndVerify(i1b, "nf-nonce");
    check("circuit nullifier public signal == independently computed", ok2 && ps2[3] === e1b.nullifier.toString());
  }

  console.log(`\n${failed === 0 ? "ALL GREEN" : "FAILURES"} — passed=${passed} failed=${failed}`);
  process.exit(failed === 0 ? 0 : 1);
}

main().catch((e) => { console.error("test-consent FAILED:", e); process.exit(1); });
