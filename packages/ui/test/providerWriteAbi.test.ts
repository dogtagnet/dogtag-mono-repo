// The provider self-service WRITE ABIs, pinned by selector (registry-plan S-15).
//
// WHY THIS EXISTS. No flow's send path has ever been executed against a chain - it cannot be, until
// a provider is registered and approved, which is C-2 registrar KYC work. So the ABIs the four
// flows encode against are the one part of this slice with no empirical check behind them, and a
// wrong ABI fails in the worst available way: a mistyped argument type or name produces a DIFFERENT
// selector, which no compiler catches, and the transaction reverts on chain for a reason that reads
// like an authorization problem rather than a typo.
//
// That is the same failure mode this repo already records for the mobile clients, where a stale
// hard-coded `isValid` selector reverted on every deployed clone and made every validity read fall
// through to "unknown". The remedy there was to DERIVE the selector and pin it against `cast sig`;
// this is that remedy applied to a hand-written ABI.
//
// The expected values below were produced by `cast sig "<signature>"` against the canonical
// signatures in contracts/src/{DogTagIssuerFactoryV2,ProviderRegistry,ServiceDomainResolver,
// ProviderDirectory}.sol. Do not recompute them from the ABI under test - that would be the ABI
// agreeing with itself.
import { toFunctionSelector } from "viem";
import { describe, expect, it } from "vitest";
import {
  CORE_ABI,
  DIRECTORY_ABI,
  DOMAIN_ABI,
  FACTORY_ABI,
} from "../src/provider/liveReader";

/** name -> the selector `cast sig` gives for the contract's own signature. */
const EXPECTED: Readonly<Record<string, `0x${string}`>> = {
  // writes - the never-executed path, and the reason this file exists
  createIssuer: "0x63478fe8",
  repointService: "0x2938ad9c",
  claimDomain: "0x89f4ae57",
  declareNoDomain: "0x6e13527f",
  clearDomain: "0xf0963cd7",
  setProfileAnchor: "0x4baa4a1f",
  publishPin: "0xe22b7b87",
  // the two reads whose argument shapes are easiest to get wrong
  canCreateService: "0xecacdfbc",
  isClone: "0x00ae3676",
  predictIssuer: "0xa3604e4f",
};

const ALL = [...FACTORY_ABI, ...CORE_ABI, ...DOMAIN_ABI, ...DIRECTORY_ABI];

describe("every ABI entry this surface sends against matches the deployed signature", () => {
  for (const [name, selector] of Object.entries(EXPECTED)) {
    it(`${name} encodes to ${selector}`, () => {
      const entry = ALL.find((e) => e.type === "function" && e.name === name);
      expect(entry, `${name} is absent from the provider ABIs`).toBeDefined();
      expect(toFunctionSelector(entry as never)).toBe(selector);
    });
  }

  it("covers every write function the flows can send", () => {
    // A selector test that silently stopped covering a function would be worse than none: the
    // uncovered one is exactly where a typo would then live. So the write list is asserted as a
    // SET, not merely iterated.
    const writes = ALL.filter(
      (e) => e.type === "function" && e.stateMutability === "nonpayable",
    ).map((e) => (e as { name: string }).name);
    expect(new Set(writes)).toEqual(
      new Set([
        "createIssuer",
        "repointService",
        "claimDomain",
        "declareNoDomain",
        "clearDomain",
        "setProfileAnchor",
        "publishPin",
      ]),
    );
  });

  it("declares no write this surface does not intend to send", () => {
    // The inverse guard. An extra `nonpayable` entry is a transaction someone can reach for, and an
    // ABI is the only thing standing between a page and a call it was never meant to make.
    const writes = ALL.filter((e) => e.type === "function" && e.stateMutability === "nonpayable");
    expect(writes).toHaveLength(7);
  });
});
