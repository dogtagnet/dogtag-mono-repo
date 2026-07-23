// Generate the shared consent-vectors.json (impl §1.10) — keyHash = Poseidon(Ax, Ay) vectors.
// TS is the reference; crates/dogtag-standard-rs/tests/consent_parity.rs asserts THIS file so the
// keyHash the owner-hidden profile tree commits is byte-identical across languages (impl §9).
import {writeFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {keyHash} from "../src/index.js";

const __dirname = dirname(fileURLToPath(import.meta.url));

// keyHash vectors — Poseidon(Ax, Ay) over the BN254 scalar field.
const keyHashVecs = [
  {name: "kh_1_2", Ax: "1", Ay: "2", expected: keyHash(1n, 2n)},
  {
    name: "kh_large",
    Ax: "12345678901234567890123456789012345678901234567890",
    Ay: "98765432109876543210987654321098765432109876543210",
    expected: keyHash(
      12345678901234567890123456789012345678901234567890n,
      98765432109876543210987654321098765432109876543210n,
    ),
  },
];

const out = {
  _comment:
    "DogTag consent-key test vectors (impl §1.10). TS = reference; dogtag-standard-rs asserts " +
    "this file. keyHash = Poseidon(Ax, Ay) — the value the owner.consentKey leaf commits into R.",
  keyHash: keyHashVecs,
};

const path = resolve(__dirname, "..", "consent-vectors.json");
writeFileSync(path, JSON.stringify(out, null, 2) + "\n");
console.log(`wrote ${path}: ${keyHashVecs.length} keyHash vectors`);
