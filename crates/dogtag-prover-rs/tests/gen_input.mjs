// Test helper: emit the EXACT circuit input object that circuits/scripts/gen-zk-fixture.mjs
// builds (numLeaves=13, dogTagId=424242, ...), plus the expected public-signal vector in
// decimal, as a single JSON object on stdout. Consumed by tests/prove.rs.
//
// Module resolution: poseidon-lite + circomlibjs live in circuits/node_modules and the SDK
// dist is under packages/. ESM resolves bare imports from THIS file's location (not cwd),
// so everything is loaded by absolute path off MONOREPO_ROOT instead.
import {resolve} from "node:path";

const ROOT = process.env.MONOREPO_ROOT;
if (!ROOT) throw new Error("MONOREPO_ROOT env not set");
const CNM = resolve(ROOT, "circuits", "node_modules");
const {poseidon2, poseidon3, poseidon5, poseidon6} = await import(resolve(CNM, "poseidon-lite", "index.js"));
const {buildEddsa} = await import(resolve(CNM, "circomlibjs", "main.js"));
const SDK = resolve(ROOT, "packages", "dogtag-standard-ts", "dist");
const {buildMerkle} = await import(resolve(SDK, "merkle.js"));
const {DS_LEAF, DS_NULLIFIER} = await import(resolve(SDK, "field.js"));

const N = 24;
const NUM_LEAVES = 13;

const poseidon = (a) => ({2: poseidon2, 3: poseidon3, 5: poseidon5, 6: poseidon6}[a.length](a));
const hashLeaf = (kp, salt, tag, val) => poseidon([DS_LEAF, kp, salt, tag, val]);

const eddsa = await buildEddsa();
const F = eddsa.babyJub.F;

const dogTagId = 424242n;
const purpose = 7n;
const relayer = 0x1111111111111111111111111111111111111111n;
const subject = 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdfn;
const consentNonce = 99n;
const dogTagKeyPathField = 0xabcdef1234567890n;
const dogTagIdLeafIndex = 3;

const kp = [], salt = [], tag = [], val = [];
for (let i = 0; i < N; i++) {
  if (i === dogTagIdLeafIndex) { kp.push(dogTagKeyPathField); salt.push(1000n + BigInt(i)); tag.push(2n); val.push(dogTagId); }
  else { kp.push(BigInt(100 + i) * 7919n); salt.push(1000n + BigInt(i)); tag.push(BigInt((i % 4) + 1)); val.push(BigInt(500000 + i * 13)); }
}
const leaves = val.map((_, i) => hashLeaf(kp[i], salt[i], tag[i], val[i]));
const active = leaves.slice(0, NUM_LEAVES);
const sdk = buildMerkle(active);
const sortedActive = sdk.layers[0];
const R = sdk.root;
const sortedLeafHashes = [], perm = [];
for (let k = 0; k < N; k++) {
  if (k < NUM_LEAVES) { sortedLeafHashes.push(sortedActive[k]); perm.push(active.findIndex((x) => x === sortedActive[k])); }
  else { sortedLeafHashes.push(leaves[k]); perm.push(k); }
}

const M = poseidon([dogTagId, purpose, relayer, subject, R, consentNonce]);
const prv = Buffer.from("0001020304050607080900010203040506070809000102030405060708090001", "hex");
const pub = eddsa.prv2pub(prv);
const Ax = F.toObject(pub[0]), Ay = F.toObject(pub[1]);
const sig = eddsa.signPoseidon(prv, F.e(M));
const nullifier = poseidon([DS_NULLIFIER, dogTagId, purpose, relayer, subject, consentNonce]);
const keyHash = poseidon([Ax, Ay]);

const input = {
  dogTagId: String(dogTagId), purpose: String(purpose), relayer: String(relayer), subject: String(subject),
  numLeaves: String(NUM_LEAVES),
  leafKeyPathHashes: kp.map(String), leafTypeTags: tag.map(String), leafSalts: salt.map(String), leafValues: val.map(String),
  dogTagIdLeafIndex: String(dogTagIdLeafIndex), sortedLeafHashes: sortedLeafHashes.map(String), perm: perm.map(String),
  dogTagKeyPathField: String(dogTagKeyPathField), consentNonce: String(consentNonce),
  Ax: String(Ax), Ay: String(Ay), R8x: String(F.toObject(sig.R8[0])), R8y: String(F.toObject(sig.R8[1])), S: String(sig.S),
};

// snarkjs public-signal order = [dogTagId, purpose, relayer, subject, nullifier, keyHash, R]
const pubDecimal = [dogTagId, purpose, relayer, subject, nullifier, keyHash, R].map(String);

process.stdout.write(JSON.stringify({input, pubDecimal}));
