// Generate the shared consent-vectors.json (impl §11.8/§11.9) — VerificationConsent inputs ->
// expected typehash, domainSeparator, EIP-712 digest, Poseidon nullifier, eddsa message, keyHash.
// TS is the reference; crates/dogtag-standard-rs/tests/consent_parity.rs asserts THIS file so the
// EIP-712 digest / nullifier / keyHash are byte-identical across languages (impl §9).
import {writeFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {
  VERIFICATION_CONSENT_TYPEHASH,
  VERIFICATION_CONSENT_TYPE_STRING,
  DOGTAG_CHAIN_ID,
  domainSeparator,
  hashTypedConsent,
  consentNullifier,
  eddsaConsentMessage,
  keyHash,
  type VerificationConsent,
} from "../src/index.js";

const __dirname = dirname(fileURLToPath(import.meta.url));

// A fixed verifyingContract (VerificationRegistry) + chainId 135 for reproducible domain separators.
const VERIFYING_CONTRACT = "0x00000000000000000000000000000000000000a7";

function b32(byte: number): string {
  return "0x" + byte.toString(16).padStart(2, "0").repeat(32);
}
function addr(byte: number): string {
  return "0x" + byte.toString(16).padStart(2, "0").repeat(20);
}

interface ConsentVec {
  name: string;
  consent: {
    dogTagId: string;
    recordType: string;
    purpose: string;
    credentialRoot: string;
    challenge: string;
    relayer: string;
    subject: string;
    nonce: string;
    deadline: string;
  };
  eip712_digest: string;
  nullifier: string;
  eddsa_message_dec: string;
}

function vec(name: string, c: VerificationConsent): ConsentVec {
  return {
    name,
    consent: {
      dogTagId: c.dogTagId.toString(),
      recordType: c.recordType,
      purpose: c.purpose,
      credentialRoot: c.credentialRoot,
      challenge: c.challenge,
      relayer: c.relayer,
      subject: c.subject,
      nonce: c.nonce.toString(),
      deadline: c.deadline.toString(),
    },
    eip712_digest: hashTypedConsent(c, VERIFYING_CONTRACT, DOGTAG_CHAIN_ID),
    nullifier: consentNullifier(c),
    eddsa_message_dec: eddsaConsentMessage(c).toString(),
  };
}

// Vector 1 — the poseidon-gate anchor (purpose=7, relayer=0x11..,subject=0x22..,dogTagId=42,nonce=99).
// Its nullifier MUST equal circuits/poseidon-vectors.json `nullifier_basic`.
const anchor: VerificationConsent = {
  dogTagId: 42n,
  recordType: b32(0),
  purpose: "0x" + "00".repeat(31) + "07",
  credentialRoot: b32(0),
  challenge: b32(0),
  relayer: addr(0x11),
  subject: addr(0x22),
  nonce: 99n,
  deadline: 0n,
};

// Vector 2 — a fully-populated realistic consent.
const populated: VerificationConsent = {
  dogTagId: 123456789n,
  recordType: b32(0xaa),
  purpose: b32(0xbb),
  credentialRoot: "0x055077ae7cbe2e123ad701247450fa222fabe3d3b399bfd40f416da970cfca11",
  challenge: b32(0xcc),
  relayer: addr(0xde),
  subject: addr(0xbe),
  nonce: 7n,
  deadline: 1893456000n, // 2030-01-01
};

// Vector 3 — large uint256 fields + a purpose that exceeds r (must reduce mod r in the nullifier).
const big: VerificationConsent = {
  dogTagId: (1n << 200n) + 1n,
  recordType: b32(0x01),
  purpose: "0x" + "ff".repeat(32), // > r -> reduced mod r for Poseidon, raw for EIP-712
  credentialRoot: "0x" + "ff".repeat(32),
  challenge: b32(0x02),
  relayer: addr(0xff),
  subject: addr(0x01),
  nonce: (1n << 255n) + 5n,
  deadline: (1n << 64n) - 1n,
};

const vectors = [vec("anchor", anchor), vec("populated", populated), vec("big", big)];

// keyHash vectors — independent of a particular consent (Poseidon(Ax, Ay)).
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
    "DogTag consent test vectors (impl §11.8/§11.9). TS = reference; dogtag-standard-rs asserts " +
    "this file. EIP-712 digest = keccak256(0x1901 || domainSeparator || structHash); " +
    "nullifier = Poseidon(4, dogTagId, purpose mod r, uint160(relayer), uint160(subject), nonce); " +
    "eddsa message = Poseidon(dogTagId, purpose, relayer, subject, credentialRoot, nonce).",
  type_string: VERIFICATION_CONSENT_TYPE_STRING,
  typehash: VERIFICATION_CONSENT_TYPEHASH,
  chain_id: DOGTAG_CHAIN_ID.toString(),
  verifying_contract: VERIFYING_CONTRACT,
  domain_separator: domainSeparator(VERIFYING_CONTRACT, DOGTAG_CHAIN_ID),
  vectors,
  keyHash: keyHashVecs,
  poseidon_gate_anchor: "0x055077ae7cbe2e123ad701247450fa222fabe3d3b399bfd40f416da970cfca11",
};

const path = resolve(__dirname, "..", "consent-vectors.json");
writeFileSync(path, JSON.stringify(out, null, 2) + "\n");
console.log(`wrote ${path}: ${vectors.length} consent, ${keyHashVecs.length} keyHash vectors`);
console.log(`typehash=${VERIFICATION_CONSENT_TYPEHASH}`);
console.log(`anchor nullifier=${vectors[0]!.nullifier}`);
