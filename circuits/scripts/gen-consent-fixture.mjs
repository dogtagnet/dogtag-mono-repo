// Generate REAL Groth16 proof fixtures for the M4 Foundry integration test
// (contracts/test/consent-fixture.json): valid (a,b,c,pub) from circuits/consent.circom against the
// PRODUCTION M3 ceremony key, plus the on-chain state values the test must establish to make
// VerificationRegistryConsent.recordVerificationZK succeed.
//
// Mirrors gen-zk-fixture.mjs (the Level-A equivalent) but for the Level-B owner-blind circuit:
//   pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]  -- no subject, no keyHash.
//
// Emits TWO proofs:
//   main  -- the happy path (a realistic VACCINATION recordType).
//   art9  -- identical except recordType = keccak256("SERVICE_ATTESTATION") % r, so the test can prove
//            the Art.9 guard actually fires on a REAL proof rather than on a hand-made pub vector.
//
// Run after the M3 ceremony artifacts exist (build/consent_final.zkey + build/consent_js/consent.wasm).
import {execFileSync} from "node:child_process";
import {existsSync, mkdtempSync, writeFileSync, rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {buildEddsa} from "circomlibjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const BUILD = resolve(ROOT, "build");
const WASM = resolve(BUILD, "consent_js", "consent.wasm");
const ZKEY = resolve(BUILD, "consent_final.zkey");

const SDK = resolve(ROOT, "..", "packages", "dogtag-standard-ts", "dist");
const {buildMerkle, merkleProof, processProof} = await import(resolve(SDK, "merkle.js"));
const {DS_LEAF, DS_NULLIFIER, FIELD_P, poseidon, fieldFromScalarBytes} =
  await import(resolve(SDK, "field.js"));
const {fieldOfKeyPath, fieldOfValue} = await import(resolve(SDK, "leaf.js"));

const DEPTH = 6; // must match `component main = DogTagConsent(6)`

// Reserved-leaf schema — MUST equal the literals pinned in consent.circom (AGENTS.md carries the table).
const KP_OWNER_ADDRESS = 20593649144631820416234157596070441856608371338897391424937040814759273231214n;
const KP_CONSENT_KEY = 7822071287675030884271946396254564996644565056920260282559292033992393086992n;
const KP_OWNER_SECRET = 11172449362271989869407103131203633198993612309996015027844083581837121079156n;
const TAG_RESERVED = 5n; // TypeTag.Bytes

const SNARKJS = existsSync(resolve(ROOT, "node_modules", ".bin", "snarkjs"))
  ? resolve(ROOT, "node_modules", ".bin", "snarkjs")
  : resolve(ROOT, "..", "node_modules", ".bin", "snarkjs");

const sh = (args, cwd = ROOT) =>
  execFileSync(SNARKJS, args, {cwd, stdio: ["ignore", "pipe", "pipe"]}).toString();
const hex32 = (x) => "0x" + BigInt(x).toString(16).padStart(64, "0");
const hashLeaf5 = (kp, salt, tag, val) => poseidon([DS_LEAF, kp, salt, tag, val]);

// `keccak256(label) % r` — how a bytes32 label enters the circuit as a field SIGNAL (see
// packages/dogtag-standard-ts/src/consent.ts: "A bytes32 reduced into [0, r)"). Precomputed rather
// than derived, because the circuits workspace ships no keccak (circomlibjs/poseidon-lite are
// Poseidon-only) and pulling one in for two constants is not worth the dependency. These are NOT
// trusted blind: ConsentRegistry.t.sol recomputes `uint256(keccak256(label)) % r` natively in
// Solidity and asserts it equals the proof's pub[1]/pub[5], so a wrong value here fails the test.
const LABEL_FIELD = {
  GROOMING_INTAKE:
    19468596258208407494832095966190273484802776478408793538485276668808211922448n,
  VACCINATION: 1936216899938869967619264895862771489778324292781763688814239930628455400799n,
  SERVICE_ATTESTATION:
    10025591956217394737855806998434905929145386518960477508456501950324730293568n,
};
const labelField = (s) => {
  const v = LABEL_FIELD[s];
  if (v === undefined) throw new Error(`no precomputed field for label ${s}`);
  return v;
};

function saltField(seed) {
  const b = new Uint8Array(16);
  for (let i = 0; i < 16; i++) b[i] = (seed * 31 + i * 7 + 1) & 0xff;
  return fieldFromScalarBytes(b);
}

/** SDK `Sibling | Promote` proof -> circuit's front-packed sibling array + length. */
function padPath(proof) {
  const siblings = proof
    .filter((s) => typeof s !== "object" || "sibling" in s)
    .map((s) => (typeof s === "object" ? s.sibling : s).toString());
  if (siblings.length > DEPTH) throw new Error(`path too deep (${siblings.length} > ${DEPTH})`);
  const pathLen = siblings.length;
  while (siblings.length < DEPTH) siblings.push("0");
  return {siblings, pathLen: String(pathLen)};
}

/** Build one honest Level-B witness: 3 reserved owner leaves + attribute leaves -> root R. */
async function buildInput(eddsa, {recordType}) {
  const F = eddsa.babyJub.F;

  // The off-chain handle (what the operator types) -> the CANONICAL on-chain dogTagId field via
  // fieldOfValue(Integer(handle)). This is what the M7 issuance path (`field-hash` / `build_profile_tree`
  // / the consent assembler) and the SBT mint all use, so the committed vector binds the CANONICAL
  // field, not the raw handle. Do NOT revert to `424242n` raw — that is the shortcut the ZK cross-check
  // (§2) and profile_tree.rs:279-287 warn against; a raw-bound fixture diverges from the minted id.
  const dogTagIdHandle = "424242";
  const dogTagId = fieldOfValue({tag: 3 /* TypeTag.Integer */, value: dogTagIdHandle});
  const purpose = labelField("GROOMING_INTAKE");
  // `relayer` is bound into BOTH the EdDSA message `M` and the nullifier, so a proof can only ever be
  // submitted by the address named here — the owner consents to one specific relayer. That makes the
  // committed fixture unsubmittable from any key we hold, which is fine for the Foundry tests (they
  // `prank`) but not for the Rust relayer E2E, which signs with a real anvil key. Hence the override:
  // point it at an anvil account to produce a fixture the backend can actually broadcast.
  // Defaults to the original constant, so `node gen-consent-fixture.mjs` with no env reproduces
  // every PUBLIC value of the committed fixture (pub, R, nullifier, recordType, deadline) — but NOT
  // the file byte-for-byte: Groth16 proving draws fresh randomness per run, so `a`/`b`/`c` differ on
  // every invocation. A diff in the proof elements is expected, not artifact drift; do not
  // regenerate a committed fixture merely to "check" it. Changing a PUBLIC INPUT does not move the
  // VK — same ceremony key, same verifier contract.
  const relayer = BigInt(process.env.CONSENT_FIXTURE_RELAYER ?? "0x1111111111111111111111111111111111111111");
  const deadline = 1893456000n; // 2030-01-01
  const consentNonce = 99n;

  // Owner private material — NONE of this may ever surface on-chain. The fixture asserts that.
  const ownerAddress = 0x00000000000000000000000000000000dEaDbeeFn;
  const ownerSecret =
    0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdefn % FIELD_P;
  const ownerSalt = saltField(11);
  const keySalt = saltField(22);
  const secretSalt = saltField(33);

  const prv = Buffer.from("0001020304050607080900010203040506070809000102030405060708090001", "hex");
  const pubKey = eddsa.prv2pub(prv);
  const Ax = F.toObject(pubKey[0]);
  const Ay = F.toObject(pubKey[1]);
  const keyHash = poseidon([Ax, Ay]);

  // M binds the consent to THIS (tag, purpose, relayer, deadline, nonce). recordType is NOT in M.
  const M = poseidon([dogTagId, purpose, relayer, deadline, consentNonce]);
  const sig = eddsa.signPoseidon(prv, F.e(M));

  const ownerLeaf = hashLeaf5(KP_OWNER_ADDRESS, ownerSalt, TAG_RESERVED, ownerAddress);
  const keyLeaf = hashLeaf5(KP_CONSENT_KEY, keySalt, TAG_RESERVED, keyHash);
  const secretLeaf = hashLeaf5(KP_OWNER_SECRET, secretSalt, TAG_RESERVED, ownerSecret);

  const attrs = [];
  for (let i = 0; i < 5; i++) {
    const kp = fieldOfKeyPath(`credentialSubject.attr${i}`);
    attrs.push(hashLeaf5(kp, saltField(100 + i), BigInt((i % 4) + 1), BigInt(700000 + i * 101)));
  }

  const tree = buildMerkle([ownerLeaf, keyLeaf, secretLeaf, ...attrs]);
  const R = tree.root;
  const ownerPath = padPath(merkleProof(tree.layers, ownerLeaf));
  const keyPath = padPath(merkleProof(tree.layers, keyLeaf));
  const secretPath = padPath(merkleProof(tree.layers, secretLeaf));
  if (processProof(merkleProof(tree.layers, secretLeaf), secretLeaf) !== R) {
    throw new Error("SDK processProof != R (harness bug)");
  }

  const nullifier = poseidon([DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce]);

  return {
    meta: {dogTagId, purpose, relayer, nullifier, R, recordType, deadline, ownerAddress},
    input: {
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
      R8x: F.toObject(sig.R8[0]).toString(),
      R8y: F.toObject(sig.R8[1]).toString(),
      S: sig.S.toString(),
      ownerSalt: ownerSalt.toString(),
      keySalt: keySalt.toString(),
      secretSalt: secretSalt.toString(),
      ownerSiblings: ownerPath.siblings,
      ownerPathLen: ownerPath.pathLen,
      keySiblings: keyPath.siblings,
      keyPathLen: keyPath.pathLen,
      secretSiblings: secretPath.siblings,
      secretPathLen: secretPath.pathLen,
    },
  };
}

/** Prove one input -> Solidity calldata (b already swapped by snarkjs). */
function prove(input) {
  const dir = mkdtempSync(resolve(tmpdir(), "dogtag-consent-fixture-"));
  try {
    const inJson = resolve(dir, "input.json");
    const wtns = resolve(dir, "w.wtns");
    const proofJson = resolve(dir, "proof.json");
    const pubJson = resolve(dir, "public.json");
    writeFileSync(inJson, JSON.stringify(input));
    sh(["wtns", "calculate", WASM, inJson, wtns], dir);
    sh(["groth16", "prove", ZKEY, wtns, proofJson, pubJson], dir);
    const raw = sh(["zkey", "export", "soliditycalldata", pubJson, proofJson], dir).trim();
    const [a, b, c, pub] = JSON.parse("[" + raw + "]");
    return {a, b, c, pub};
  } finally {
    rmSync(dir, {recursive: true, force: true});
  }
}

async function main() {
  for (const f of [WASM, ZKEY]) {
    if (!existsSync(f)) throw new Error(`missing ${f} — the M3 ceremony artifacts must be present`);
  }
  const eddsa = await buildEddsa();

  const main_ = await buildInput(eddsa, {recordType: labelField("VACCINATION")});
  const art9_ = await buildInput(eddsa, {recordType: labelField("SERVICE_ATTESTATION")});

  const p = prove(main_.input);
  const pArt9 = prove(art9_.input);
  const m = main_.meta;

  // The circuit emits pub in the M4 order; assert that here so a circuit/README drift trips the
  // generator rather than silently producing a fixture the registry mis-indexes.
  const expect = [m.dogTagId, m.purpose, m.relayer, m.nullifier, m.R, m.recordType, m.deadline];
  expect.forEach((want, i) => {
    if (BigInt(p.pub[i]) !== want) {
      throw new Error(`pub[${i}] mismatch: circuit ${BigInt(p.pub[i])} != expected ${want}`);
    }
  });

  const fixture = {
    _comment:
      "Real Groth16 proofs from circuits/consent.circom (DogTagConsent(6), 3 owner leaves + 5 attrs) " +
      "against the PRODUCTION M3 ceremony zkey. Solidity calldata (b swapped). " +
      "pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline] — no subject, no keyHash. " +
      "Regenerate: node circuits/scripts/gen-consent-fixture.mjs",
    a: p.a,
    b: p.b,
    c: p.c,
    pub: p.pub,
    // On-chain state the Foundry test must establish for recordVerificationZK to pass:
    dogTagId: String(m.dogTagId),
    purpose: hex32(m.purpose),
    relayer: "0x" + m.relayer.toString(16).padStart(40, "0"),
    nullifier: hex32(m.nullifier),
    R: hex32(m.R),
    recordType: hex32(m.recordType),
    deadline: String(m.deadline),
    // Owner material that MUST NOT appear on-chain — the test greps calldata/logs for it.
    _ownerAddress: "0x" + m.ownerAddress.toString(16).padStart(40, "0"),
    // Same witness, recordType = keccak256("SERVICE_ATTESTATION") % r -> must revert "art9".
    art9: {a: pArt9.a, b: pArt9.b, c: pArt9.c, pub: pArt9.pub},
  };

  // Output path override, so generating a relayer-bound variant can NEVER overwrite the committed
  // fixture the Foundry suite pins.
  const out = process.env.CONSENT_FIXTURE_OUT
    ? resolve(process.env.CONSENT_FIXTURE_OUT)
    : resolve(ROOT, "..", "contracts", "test", "consent-fixture.json");
  writeFileSync(out, JSON.stringify(fixture, null, 2) + "\n");
  console.log("wrote", out);
  console.log("  pub =", p.pub.map((x) => BigInt(x).toString()).join(", "));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
