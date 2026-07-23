import {describe, it, expect} from "vitest";
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {keyHash, deriveBabyjubConsentKey} from "../src/index.js";

const VECS = JSON.parse(
  readFileSync(resolve(__dirname, "..", "consent-vectors.json"), "utf8"),
);

describe("keyHash vectors", () => {
  for (const v of VECS.keyHash) {
    it(`${v.name}`, () => {
      expect(keyHash(BigInt(v.Ax), BigInt(v.Ay))).toBe(v.expected);
    });
  }
});

describe("BabyJubjub consent-key derivation (circomlibjs)", () => {
  it("derives a deterministic keypair with a stable keyHash", async () => {
    const seed = new Uint8Array(32).fill(7);
    const key = await deriveBabyjubConsentKey(seed);
    expect(typeof key.Ax).toBe("bigint");
    expect(typeof key.Ay).toBe("bigint");

    // Deterministic: the same seed re-derives the same public point + keyHash.
    const again = await deriveBabyjubConsentKey(seed);
    expect(again.Ax).toBe(key.Ax);
    expect(again.Ay).toBe(key.Ay);
    expect(keyHash(again.Ax, again.Ay)).toBe(keyHash(key.Ax, key.Ay));

    // A different seed derives a different point.
    const other = await deriveBabyjubConsentKey(new Uint8Array(32).fill(8));
    expect(other.Ax).not.toBe(key.Ax);
  });
});
