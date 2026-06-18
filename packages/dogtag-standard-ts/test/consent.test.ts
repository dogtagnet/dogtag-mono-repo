import {describe, it, expect} from "vitest";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {
  VERIFICATION_CONSENT_TYPEHASH,
  VERIFICATION_CONSENT_TYPE_STRING,
  DOGTAG_CHAIN_ID,
  domainSeparator,
  hashTypedConsent,
  consentNullifier,
  eddsaConsentMessage,
  keyHash,
  deriveBabyjubConsentKey,
  signConsentEddsa,
  type VerificationConsent,
} from "../src/index.js";

const VECS = JSON.parse(
  readFileSync(resolve(__dirname, "..", "consent-vectors.json"), "utf8"),
);

const POSEIDON_GATE_ANCHOR =
  "0x055077ae7cbe2e123ad701247450fa222fabe3d3b399bfd40f416da970cfca11";

function consentOf(v: {
  dogTagId: string;
  recordType: string;
  purpose: string;
  credentialRoot: string;
  challenge: string;
  relayer: string;
  subject: string;
  nonce: string;
  deadline: string;
}): VerificationConsent {
  return {
    dogTagId: BigInt(v.dogTagId),
    recordType: v.recordType,
    purpose: v.purpose,
    credentialRoot: v.credentialRoot,
    challenge: v.challenge,
    relayer: v.relayer,
    subject: v.subject,
    nonce: BigInt(v.nonce),
    deadline: BigInt(v.deadline),
  };
}

describe("consent EIP-712 typehash + domain", () => {
  it("type string field order matches the struct (impl §11.8(a))", () => {
    expect(VERIFICATION_CONSENT_TYPE_STRING).toBe(
      "VerificationConsent(uint256 dogTagId,bytes32 recordType,bytes32 purpose,bytes32 credentialRoot," +
        "bytes32 challenge,address relayer,address subject,uint256 nonce,uint256 deadline)",
    );
  });

  it("typehash matches the recorded vector", () => {
    expect(VERIFICATION_CONSENT_TYPEHASH).toBe(VECS.typehash);
  });

  it("domainSeparator is deterministic for chainId 135 + fixed contract", () => {
    expect(domainSeparator(VECS.verifying_contract, DOGTAG_CHAIN_ID)).toBe(
      VECS.domain_separator,
    );
    expect(DOGTAG_CHAIN_ID).toBe(135n);
  });
});

describe("consent vectors (consent-vectors.json) — digest / nullifier / eddsa message", () => {
  for (const v of VECS.vectors) {
    const c = consentOf(v.consent);
    it(`${v.name}: EIP-712 digest`, () => {
      expect(hashTypedConsent(c, VECS.verifying_contract, DOGTAG_CHAIN_ID)).toBe(
        v.eip712_digest,
      );
    });
    it(`${v.name}: nullifier`, () => {
      expect(consentNullifier(c)).toBe(v.nullifier);
    });
    it(`${v.name}: eddsa message`, () => {
      expect(eddsaConsentMessage(c).toString()).toBe(v.eddsa_message_dec);
    });
  }
});

describe("keyHash vectors", () => {
  for (const v of VECS.keyHash) {
    it(`${v.name}`, () => {
      expect(keyHash(BigInt(v.Ax), BigInt(v.Ay))).toBe(v.expected);
    });
  }
});

describe("poseidon-gate anchor sanity", () => {
  it("anchor consent nullifier == nullifier_basic in poseidon-vectors.json", () => {
    const anchor = VECS.vectors.find((x: {name: string}) => x.name === "anchor");
    expect(anchor.nullifier).toBe(POSEIDON_GATE_ANCHOR);
    // recompute directly (not just trust the file)
    const c = consentOf(anchor.consent);
    expect(consentNullifier(c)).toBe(POSEIDON_GATE_ANCHOR);
  });
});

describe("EdDSA-BabyJubjub signing (circomlibjs round-trip)", () => {
  it("derive key, sign the consent message, and verify it", async () => {
    const seed = new Uint8Array(32).fill(7);
    const key = await deriveBabyjubConsentKey(seed);
    expect(typeof key.Ax).toBe("bigint");
    expect(typeof key.Ay).toBe("bigint");

    const anchor = VECS.vectors.find((x: {name: string}) => x.name === "anchor");
    const c = consentOf(anchor.consent);
    const sig = await signConsentEddsa(c, key.prv);
    expect(sig.R8x).toMatch(/^\d+$/);
    expect(sig.R8y).toMatch(/^\d+$/);
    expect(sig.S).toMatch(/^\d+$/);

    // round-trip verify with circomlibjs
    const {buildEddsa} = await import("circomlibjs");
    const eddsa = await buildEddsa();
    const F = eddsa.F;
    const m = eddsaConsentMessage(c);
    const ok = eddsa.verifyPoseidon(
      F.e(m.toString()),
      {R8: [F.e(sig.R8x), F.e(sig.R8y)], S: BigInt(sig.S)},
      [F.e(key.Ax.toString()), F.e(key.Ay.toString())],
    );
    expect(ok).toBe(true);
  });
});
