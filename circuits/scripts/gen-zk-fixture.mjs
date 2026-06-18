// Generate a REAL Groth16 proof fixture for the Foundry integration test
// (contracts/test/zk-fixture.json): a valid (a,b,c,pub) + the on-chain state values needed to make
// VerificationRegistry.recordVerificationZK succeed against the snarkjs-generated Groth16Verifier.
// Run after `npm run build-circuit`.
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
const ZKEY = resolve(BUILD, "verification_final.zkey");
const SNARKJS = existsSync(resolve(ROOT, "node_modules", ".bin", "snarkjs"))
  ? resolve(ROOT, "node_modules", ".bin", "snarkjs")
  : resolve(ROOT, "..", "node_modules", ".bin", "snarkjs");
const SDK = resolve(ROOT, "..", "packages", "dogtag-standard-ts", "dist");
const {buildMerkle} = await import(resolve(SDK, "merkle.js"));
const {DS_LEAF, DS_NULLIFIER} = await import(resolve(SDK, "field.js"));
const N = 8;

const sh = (args, cwd = ROOT) => execFileSync(SNARKJS, args, {cwd, stdio: ["ignore", "pipe", "pipe"]}).toString();
const poseidon = (a) => ({2: poseidon2, 3: poseidon3, 5: poseidon5, 6: poseidon6}[a.length](a));
const hashLeaf = (kp, salt, tag, val) => poseidon([DS_LEAF, kp, salt, tag, val]);
const hex32 = (x) => "0x" + BigInt(x).toString(16).padStart(64, "0");

async function main() {
  for (const f of [WASM, ZKEY]) if (!existsSync(f)) throw new Error(`missing ${f} — run npm run build-circuit`);
  const eddsa = await buildEddsa();
  const F = eddsa.babyJub.F;

  const dogTagId = 424242n;
  const purpose = 7n; // GROOMING_INTAKE label reduced mod p
  const relayer = 0x1111111111111111111111111111111111111111n;
  const subject = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdfn; // = vm.addr(1), so the test binds via real ECDSA
  const consentNonce = 99n;
  const dogTagKeyPathField = 0xabcdef1234567890n;
  const dogTagIdLeafIndex = 3;

  const kp = [], salt = [], tag = [], val = [];
  for (let i = 0; i < N; i++) {
    if (i === dogTagIdLeafIndex) { kp.push(dogTagKeyPathField); salt.push(1000n + BigInt(i)); tag.push(2n); val.push(dogTagId); }
    else { kp.push(BigInt(100 + i) * 7919n); salt.push(1000n + BigInt(i)); tag.push(BigInt((i % 4) + 1)); val.push(BigInt(500000 + i * 13)); }
  }
  const leaves = val.map((_, i) => hashLeaf(kp[i], salt[i], tag[i], val[i]));
  const sdk = buildMerkle(leaves);
  const sortedLeafHashes = sdk.layers[0];
  const R = sdk.root;
  const perm = sortedLeafHashes.map((h) => leaves.findIndex((x) => x === h));

  const M = poseidon([dogTagId, purpose, relayer, subject, R, consentNonce]);
  const prv = Buffer.from("0001020304050607080900010203040506070809000102030405060708090001", "hex");
  const pub = eddsa.prv2pub(prv);
  const Ax = F.toObject(pub[0]), Ay = F.toObject(pub[1]);
  const sig = eddsa.signPoseidon(prv, F.e(M));
  const nullifier = poseidon([DS_NULLIFIER, dogTagId, purpose, relayer, subject, consentNonce]);
  const keyHash = poseidon([Ax, Ay]);

  const input = {
    dogTagId: String(dogTagId), purpose: String(purpose), relayer: String(relayer), subject: String(subject),
    leafKeyPathHashes: kp.map(String), leafTypeTags: tag.map(String), leafSalts: salt.map(String), leafValues: val.map(String),
    dogTagIdLeafIndex: String(dogTagIdLeafIndex), sortedLeafHashes: sortedLeafHashes.map(String), perm: perm.map(String),
    dogTagKeyPathField: String(dogTagKeyPathField), consentNonce: String(consentNonce),
    Ax: String(Ax), Ay: String(Ay), R8x: String(F.toObject(sig.R8[0])), R8y: String(F.toObject(sig.R8[1])), S: String(sig.S),
  };

  const dir = mkdtempSync(resolve(tmpdir(), "dogtag-fixture-"));
  try {
    const inJson = resolve(dir, "input.json"), wtns = resolve(dir, "w.wtns");
    const proofJson = resolve(dir, "proof.json"), pubJson = resolve(dir, "public.json");
    writeFileSync(inJson, JSON.stringify(input));
    sh(["wtns", "calculate", WASM, inJson, wtns], dir);
    sh(["groth16", "prove", ZKEY, wtns, proofJson, pubJson], dir);
    // soliditycalldata already applies the snarkjs->Solidity b-coordinate swap.
    const raw = sh(["zkey", "export", "soliditycalldata", pubJson, proofJson], dir).trim();
    const [a, b, c, pubSignals] = JSON.parse("[" + raw + "]");

    const fixture = {
      _comment: "Real Groth16 proof from circuits/verification.circom (N=8). Solidity calldata (b swapped).",
      a, b, c, pub: pubSignals,
      // on-chain state the Foundry test must establish to make recordVerificationZK pass:
      dogTagId: String(dogTagId),
      purpose: hex32(purpose),
      relayer: "0x" + relayer.toString(16).padStart(40, "0"),
      subject: "0x" + subject.toString(16).padStart(40, "0"),
      nullifier: hex32(nullifier),
      keyHash: hex32(keyHash),
      R: hex32(R),
    };
    const out = resolve(ROOT, "..", "contracts", "test", "zk-fixture.json");
    writeFileSync(out, JSON.stringify(fixture, null, 2) + "\n");
    console.log("wrote", out);
    console.log("  pub =", pubSignals.map((x) => BigInt(x).toString()).join(", "));
  } finally {
    rmSync(dir, {recursive: true, force: true});
  }
}
main().catch((e) => { console.error(e); process.exit(1); });
