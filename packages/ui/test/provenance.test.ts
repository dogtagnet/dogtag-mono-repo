import { describe, expect, it } from "vitest";
import {
  addressExplorerHref,
  chainProvenance,
  emittingContractRole,
  formatChainTime,
  formatRelativeTime,
  isEvmAddress,
  isHash32,
  shortHex,
  txExplorerHref,
} from "../src/chain/provenance";

/** A well-formed 32-byte transaction hash, as a real ROAX transaction would carry. */
const REAL_TX = `0x${"ab".repeat(32)}`;
/** What the indexer's scripted demo feed actually emits (`INDEXER_DEMO_MODE` MemLogSource). */
const DEMO_TX = "0x0800";

describe("chain-addressability", () => {
  it("accepts only a 0x-prefixed 32-byte hex value as a hash", () => {
    expect(isHash32(REAL_TX)).toBe(true);
    expect(isHash32(REAL_TX.toUpperCase().replace("0X", "0x"))).toBe(true);
    expect(isHash32(DEMO_TX)).toBe(false);
    expect(isHash32(`0x${"ab".repeat(31)}`)).toBe(false); // 31 bytes
    expect(isHash32(`0x${"ab".repeat(33)}`)).toBe(false); // 33 bytes
    expect(isHash32(`0x${"zz".repeat(32)}`)).toBe(false); // not hex
    expect(isHash32("ab".repeat(32))).toBe(false); // missing 0x
    expect(isHash32(undefined)).toBe(false);
  });

  it("accepts only a 0x-prefixed 20-byte hex value as an address", () => {
    expect(isEvmAddress("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F")).toBe(true);
    expect(isEvmAddress(REAL_TX)).toBe(false); // a hash is not an address
    expect(isEvmAddress("0x0800")).toBe(false);
    expect(isEvmAddress(null)).toBe(false);
  });
});

describe("chainProvenance", () => {
  it("calls a well-formed transaction hash on-chain", () => {
    expect(chainProvenance({ txHash: REAL_TX })).toBe("onchain");
  });

  it("calls the demo indexer's scripted hashes synthetic", () => {
    expect(chainProvenance({ txHash: DEMO_TX })).toBe("synthetic");
    expect(chainProvenance({ txHash: "0xgovtx1111" })).toBe("synthetic");
    expect(chainProvenance({})).toBe("synthetic");
  });
});

describe("txExplorerHref", () => {
  it("prefers the indexer-supplied txUrl for a real transaction", () => {
    const href = txExplorerHref({ txHash: REAL_TX, txUrl: "https://other.example/tx/x" });
    expect(href).toBe("https://other.example/tx/x");
  });

  it("composes a ROAX explorer URL only when the API sent none", () => {
    expect(txExplorerHref({ txHash: REAL_TX })).toBe(`https://explorer.roax.net/tx/${REAL_TX}`);
  });

  it("refuses to link a synthetic event even when the API supplied a txUrl", () => {
    // This is the live failure: the indexer composes txUrl unconditionally, so demo rows arrive
    // carrying an explorer link to a transaction that cannot exist.
    expect(
      txExplorerHref({ txHash: DEMO_TX, txUrl: "https://explorer.roax.net/tx/0x0800" }),
    ).toBeNull();
    expect(txExplorerHref({})).toBeNull();
  });
});

describe("addressExplorerHref", () => {
  it("links a well-formed address and refuses anything else", () => {
    expect(addressExplorerHref("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F")).toBe(
      "https://explorer.roax.net/address/0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F",
    );
    expect(addressExplorerHref("0x08")).toBeNull();
    expect(addressExplorerHref(undefined)).toBeNull();
  });
});

describe("shortHex", () => {
  it("middle-truncates long values and leaves short ones intact", () => {
    expect(shortHex(REAL_TX)).toBe(`0xabababab…ababab`);
    expect(shortHex("0x0800")).toBe("0x0800");
    expect(shortHex(undefined)).toBe("—");
  });
});

describe("time formatting", () => {
  it("renders an absolute timestamp and an em dash when absent", () => {
    expect(formatChainTime(0)).toBe("—");
    expect(formatChainTime(undefined)).toBe("—");
    expect(formatChainTime(1_700_000_000)).toContain("2023");
  });

  it("renders a coarse relative distance in both directions", () => {
    const now = 1_700_000_000_000;
    expect(formatRelativeTime(1_700_000_000 - 30, now)).toBe("30s ago");
    expect(formatRelativeTime(1_700_000_000 - 600, now)).toBe("10m ago");
    expect(formatRelativeTime(1_700_000_000 - 7200, now)).toBe("2h ago");
    expect(formatRelativeTime(1_700_000_000 - 172_800, now)).toBe("2d ago");
    expect(formatRelativeTime(1_700_000_000 + 3600, now)).toBe("in 1h");
    expect(formatRelativeTime(undefined, now)).toBe("");
  });
});

describe("emittingContractRole", () => {
  it("names the contract that actually emitted each event kind", () => {
    // The emitting contract is NOT always the issuer clone - that distinction is the whole point.
    expect(emittingContractRole("issuerCreated")).toBe("issuer factory");
    expect(emittingContractRole("whitelisted")).toBe("issuer registry");
    expect(emittingContractRole("delisted")).toBe("issuer registry");
    expect(emittingContractRole("verified")).toBe("verification registry");
    expect(emittingContractRole("rootIssued")).toBe("issuer clone");
    expect(emittingContractRole("rootRevoked")).toBe("issuer clone");
    expect(emittingContractRole("somethingNew")).toBe("contract");
  });
});
