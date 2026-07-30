import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  ROAX_CHAIN_ID_HEX,
  roax,
  roaxAddChainParams,
} from "../src/wallet/chain";
import {
  ROAX_RPC_STORAGE_KEY,
  getRoaxRpcPreference,
  type StorageLike,
} from "../src/chain/rpcEndpoint";

/**
 * The chain-add payload must name the PORTAL-BUNDLED endpoint, never the browser's DogTag read
 * preference.
 *
 * The endpoint preference is scoped to DogTag's own direct reads, where the chain guard re-runs
 * immediately before every request. `wallet_addEthereumChain` is a different thing entirely: the URL
 * becomes the wallet's OWN persistent chain configuration and then serves the wallet's traffic -
 * transaction broadcast included - under a guard that could only ever have run once, at add time.
 * So a regression here does not merely widen a read; it hands a user-typed peer the wallet's writes.
 *
 * That property was asserted only by a source comment, which is why it is pinned here.
 */

class MemoryStorage implements StorageLike {
  readonly values = new Map<string, string>();
  getItem(key: string) {
    return this.values.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
  removeItem(key: string) {
    this.values.delete(key);
  }
}

const BUNDLED = roax.rpcUrls.default.http[0];
const PORTAL_BUNDLED = "https://portal-bundled.rpc.test/v1";
const HOLDER_CHOICE = "https://holder-chosen.rpc.test/v1?token=AbCdEf";

/** Simulate a browser whose localStorage already holds a custom read preference. */
function withBrowserPreference(custom: string): void {
  const storage = new MemoryStorage();
  storage.setItem(ROAX_RPC_STORAGE_KEY, custom);
  (globalThis as { window?: unknown }).window = { localStorage: storage };
}

afterEach(() => {
  delete (globalThis as { window?: unknown }).window;
});

describe("wallet_addEthereumChain metadata", () => {
  it("defaults to the bundled endpoint and takes the portal's bundled value verbatim", () => {
    expect(roaxAddChainParams()).toEqual({
      chainId: ROAX_CHAIN_ID_HEX,
      chainName: "ROAX",
      nativeCurrency: { name: "Plasma", symbol: "PLASMA", decimals: 18 },
      rpcUrls: [BUNDLED],
      blockExplorerUrls: ["https://explorer.roax.net"],
    });

    // A deployment whose VITE_ROAX_RPC differs still hands the wallet ITS OWN bundled endpoint -
    // this is the value `useRoaxChain(defaultRpcUrl)` threads through from portal env.
    expect(roaxAddChainParams(PORTAL_BUNDLED).rpcUrls).toEqual([PORTAL_BUNDLED]);
  });

  it("ignores a persisted holder endpoint choice that is genuinely in effect for reads", () => {
    withBrowserPreference(HOLDER_CHOICE);

    // Control: the preference really is active in this simulated browser, so a payload built from
    // it would differ. Without this the next assertion could pass for the wrong reason.
    expect(getRoaxRpcPreference(PORTAL_BUNDLED)).toEqual({
      rpcUrl: HOLDER_CHOICE,
      defaultRpcUrl: PORTAL_BUNDLED,
      isCustom: true,
    });

    expect(roaxAddChainParams(PORTAL_BUNDLED).rpcUrls).toEqual([PORTAL_BUNDLED]);
    expect(roaxAddChainParams().rpcUrls).toEqual([BUNDLED]);
  });

  it("keeps the preference module out of the chain-add call path", () => {
    // The behavioural assertions above hold for today's implementation; this pins the structural
    // reason they cannot quietly stop holding. `useRoaxChain` is the sole caller, and the only URL
    // in its scope is the bundled one precisely because it never imports the preference store.
    const source = (relative: string) =>
      readFileSync(fileURLToPath(new URL(relative, import.meta.url)), "utf8");
    for (const module of ["../src/wallet/useRoaxChain.ts", "../src/wallet/chain.ts"]) {
      const text = source(module);
      expect(text, `${module} must not read the endpoint preference`).not.toMatch(
        /from\s+["'][^"']*chain\/rpcEndpoint["']|getRoaxRpcPreference|useRoaxRpcPreference|useRoaxRpcSettings|ROAX_RPC_STORAGE_KEY/,
      );
    }
  });
});
