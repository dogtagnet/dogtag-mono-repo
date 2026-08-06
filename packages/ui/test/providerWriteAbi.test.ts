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
import { readFileSync } from "node:fs";
import { toFunctionSelector } from "viem";
import { describe, expect, it } from "vitest";
import {
  CORE_ABI,
  DIRECTORY_ABI,
  DOMAIN_ABI,
  FACTORY_ABI,
  PROVIDER_PERMISSION_DIRECTORY_RESOLVER,
  PROVIDER_PERMISSION_RECORD,
  SERVICE_PERMISSION_DOMAIN_RESOLVER,
  SERVICE_PERMISSION_REPOINT,
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
  // THE PROVIDER'S OWN HALF OF THE TYPED-RESOLVER PAIR. `setDirectoryResolver`'s first argument is
  // `bytes20`, not `address` - both are 20 bytes and both take the same hex string, so nothing at the
  // type level or in the wallet tells them apart, but `setDirectoryResolver(address,address)` is
  // `0xa812c18c` and reverts at the dispatcher, which reads exactly like the authorization failure
  // this whole flow is about. That is the same shape as the stale mobile `isValid` selector this repo
  // already paid for.
  setDirectoryResolver: "0x745ba9c5",
  setDomainResolver: "0x7a2a9bad",
  // The two that keep a re-publish from adding a SECOND live pin. Confirmed callable on the
  // deployed ProviderDirectory: an `eth_call` to either reverts with the NAMED `UnknownProvider()`
  // (`0xf2b51dfc`) for an unregistered provider id, while a deliberately nonexistent selector on the
  // same contract returns empty data - so a named error is positive evidence of dispatch.
  updatePin: "0xa87e91e5",
  removePin: "0x33c60a69",
  // the reads whose argument shapes are easiest to get wrong
  canCreateService: "0xecacdfbc",
  canWriteService: "0x2c337d53",
  isClone: "0x00ae3676",
  predictIssuer: "0xa3604e4f",
  profileAnchor: "0x88704a5b",
  pinCount: "0xcd0f8acd",
  nextLocationNumber: "0x51b047cd",
  hasPin: "0x2f792e58",
  pin: "0xb2d3139c",
  // The reads the register choice list is built from. `resolverPage` is the one whose shape is easy
  // to get wrong, because it returns a TUPLE and takes three arguments in a fixed order.
  isResolverApproved: "0x4e1a94b9",
  resolverCount: "0x260e1ea0",
  resolverPage: "0x8fdcab23",
  canWriteProvider: "0x415cb2e4",
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
        "updatePin",
        "removePin",
        "setDirectoryResolver",
        "setDomainResolver",
      ]),
    );
  });

  it("declares no write this surface does not intend to send", () => {
    // The inverse guard. An extra `nonpayable` entry is a transaction someone can reach for, and an
    // ABI is the only thing standing between a page and a call it was never meant to make.
    const writes = ALL.filter((e) => e.type === "function" && e.stateMutability === "nonpayable");
    expect(writes).toHaveLength(11);
  });
});

// -------------------------------------------------------------------------------------------------
// The permission bits, pinned against the contract's own declarations
// -------------------------------------------------------------------------------------------------

// A permission bit is as silent as a selector when it is wrong, and worse to diagnose: the call
// succeeds and answers a confident `false` about a permission nobody asked about, which a provider
// reads as "you may not" rather than as a mistake. So the two mirrored literals are checked against
// the source that owns them rather than against a comment beside them.
const PROVIDER_REGISTRY_SOURCE = readFileSync(
  new URL("../../../contracts/src/ProviderRegistry.sol", import.meta.url),
  "utf8",
);

/** `uint32 public constant <NAME> = 1 << N;` -> the value. */
function declaredBit(name: string): number {
  const m = PROVIDER_REGISTRY_SOURCE.match(
    new RegExp(`uint32\\s+public\\s+constant\\s+${name}\\s*=\\s*1\\s*<<\\s*(\\d+)\\s*;`),
  );
  if (!m) throw new Error(`${name} is not declared in ProviderRegistry.sol as a 1 << N constant`);
  return 1 << Number(m[1]);
}

describe("the mirrored permission bits are the contract's own", () => {
  it("PROVIDER_PERMISSION_RECORD matches its declaration", () => {
    expect(PROVIDER_PERMISSION_RECORD).toBe(declaredBit("PROVIDER_PERMISSION_RECORD"));
  });

  it("SERVICE_PERMISSION_REPOINT matches its declaration", () => {
    expect(SERVICE_PERMISSION_REPOINT).toBe(declaredBit("SERVICE_PERMISSION_REPOINT"));
  });

  it("PROVIDER_PERMISSION_DIRECTORY_RESOLVER matches its declaration", () => {
    expect(PROVIDER_PERMISSION_DIRECTORY_RESOLVER).toBe(
      declaredBit("PROVIDER_PERMISSION_DIRECTORY_RESOLVER"),
    );
  });

  it("SERVICE_PERMISSION_DOMAIN_RESOLVER matches its declaration", () => {
    expect(SERVICE_PERMISSION_DOMAIN_RESOLVER).toBe(
      declaredBit("SERVICE_PERMISSION_DOMAIN_RESOLVER"),
    );
  });

  it("keeps the two resolver bits apart from the two already mirrored here", () => {
    // THE HAZARD IS THAT THE TWO RESOLVER BITS EQUAL EACH OTHER (both `1 << 1`) AND NEITHER
    // NEIGHBOUR. So a copy-paste from `PROVIDER_PERMISSION_RECORD` (1) or `SERVICE_PERMISSION_REPOINT`
    // (4) is wrong and silent: the read succeeds and answers a confident `false` about a permission
    // nobody asked about, which a provider reads as "you may not choose a register".
    expect(PROVIDER_PERMISSION_DIRECTORY_RESOLVER).not.toBe(PROVIDER_PERMISSION_RECORD);
    expect(SERVICE_PERMISSION_DOMAIN_RESOLVER).not.toBe(SERVICE_PERMISSION_REPOINT);
    // They agree with each other TODAY, in two independent bitmask namespaces. Asserted so the
    // coincidence is recorded rather than relied on: if either namespace moves, this goes red and
    // whoever moved it has to decide, instead of one shared constant silently following the other.
    expect(PROVIDER_PERMISSION_DIRECTORY_RESOLVER).toBe(SERVICE_PERMISSION_DOMAIN_RESOLVER);
  });

  it("is not the neighbouring service bits - a delegate trusted with one is not trusted with these", () => {
    // The bits are a grant model, not a numbering. `SERVICE_PERMISSION_RECORD` publishes content and
    // `SERVICE_PERMISSION_DOMAIN_RESOLVER` chooses a resolver; neither says anything about where new
    // credentials anchor, which is what a repoint moves.
    expect(SERVICE_PERMISSION_REPOINT).not.toBe(declaredBit("SERVICE_PERMISSION_RECORD"));
    expect(SERVICE_PERMISSION_REPOINT).not.toBe(declaredBit("SERVICE_PERMISSION_DOMAIN_RESOLVER"));
  });
});
