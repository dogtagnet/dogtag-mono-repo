/**
 * @vitest-environment jsdom
 */
// The provider id survives a refresh (registry-plan S-15).
//
// THE STATE THAT SHIPPED THIS: `providerId` lived in `useState` alone, so a refresh emptied it,
// three flows went dead at once, and the reasons under their buttons pointed at an empty field in
// a card the operator had scrolled past. A captain hit it and named the general truth - "such
// things are inevitable". The id is a stable identity DogTag assigned, not a transient input, so
// it is remembered per wallet in localStorage and restored when the wallet arrives.
//
// Mounted rather than asserted on the pure module alone, because the claim is about what a REFRESH
// leaves on screen - a remount of the component against the same browser storage - and about the
// buttons coming back, which no pure test can see.
//
// Real DOM, real macrotask turns, no `act()` - the convention `rpcSettingsVerdict.test.ts`
// established in this package.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderSelfServiceFlows } from "../src/domain/ProviderSelfServiceFlows";
import {
  PROVIDER_ID_STORAGE_PREFIX,
  providerIdStorageKey,
  recallProviderId,
  rememberProviderId,
  type ProviderContracts,
} from "../src/provider";

// Mutable, so a wallet switch and a refresh-with-reconnect can each be driven from the account
// state the component actually reads.
const account: { address?: string; isConnected: boolean; chainId?: number } = {
  address: undefined,
  isConnected: false,
};
vi.mock("wagmi", () => ({
  useAccount: () => account,
  useWriteContract: () => ({ writeContractAsync: async () => "0x" }),
}));

const WALLET_A = "0x2222222222222222222222222222222222222222";
const WALLET_B = "0x3333333333333333333333333333333333333333";
const PROVIDER_A = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const PROVIDER_B = "0x2ee0cd9517174eb28df3523bb5791767d18b4ecc";

const CONTRACTS: ProviderContracts = {
  core: "0x9309aB1c2d3E4F5061728394A5B6C7D8E9F00112",
  factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607",
  domainResolver: "0x7A9b0C1d2E3F4a5B6C7d8e9f0A1B2c3D4e5F6A7b",
  directory: "0x8b0c1D2E3f4a5B6c7D8E9F0A1B2C3d4E5f6A7b8C",
};

let root: Root | null = null;
let host: HTMLElement | null = null;

const turn = () => new Promise((r) => setTimeout(r, 0));

async function mount() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(
    createElement(ProviderSelfServiceFlows, {
      contracts: CONTRACTS,
      missingConfig: [],
      capabilities: { issuance: true, listing: true },
    }),
  );
  await turn();
  await turn();
  return host;
}

/** Tear the mounted page down while the browser storage survives - which is what a refresh IS. */
function refresh() {
  root?.unmount();
  host?.remove();
  root = null;
  host = null;
}

function type(id: string, value: string) {
  const input = host!.querySelector<HTMLInputElement>(`#${id}`)!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

const providerIdField = () => host!.querySelector<HTMLInputElement>("#providerId")!;
const buttonLabelled = (label: string) =>
  Array.from(host!.querySelectorAll<HTMLButtonElement>("button")).find((b) =>
    b.textContent?.includes(label),
  )!;

beforeEach(() => {
  account.address = WALLET_A;
  account.isConnected = true;
  account.chainId = 135;
});

afterEach(() => {
  refresh();
  account.address = undefined;
  account.isConnected = false;
  account.chainId = undefined;
  window.localStorage.clear();
});

describe("a refresh does not empty the provider id", () => {
  it("restores what was typed, and the flow-2 buttons come back with it", async () => {
    await mount();
    type("providerId", PROVIDER_A);
    await turn();

    refresh();
    await mount();

    // The field the captain had filled by hand is filled again by the page.
    expect(providerIdField().value).toBe(PROVIDER_A);

    // And the state he was stranded in cannot recur from a refresh alone: with the contract
    // address re-entered, flow 2's Check is offerable rather than dead over a nameless reason.
    type("candidate", "0x1111111111111111111111111111111111111111");
    await turn();
    expect(buttonLabelled("Check this contract").disabled).toBe(false);
    expect(host!.querySelector("[data-testid='repoint-check-reason']")).toBeNull();
  });

  it("restores nothing into a field the operator has already typed into", async () => {
    // The remembered value is a convenience, and typing wins over it - the `nonceEdited` rule. A
    // restore that overwrote live typing would be deciding whose record this is on the operator's
    // behalf.
    window.localStorage.setItem(providerIdStorageKey(WALLET_A), PROVIDER_A);
    account.address = undefined;
    account.isConnected = false;
    await mount();
    type("providerId", PROVIDER_B);
    await turn();

    // The wallet arrives AFTER the typing - the reconnect race a refresh actually runs.
    account.address = WALLET_A;
    account.isConnected = true;
    root!.render(
      createElement(ProviderSelfServiceFlows, {
        contracts: CONTRACTS,
        missingConfig: [],
        capabilities: { issuance: true, listing: true },
      }),
    );
    await turn();
    await turn();

    expect(providerIdField().value).toBe(PROVIDER_B);
    // And what was typed is now remembered under the wallet that arrived.
    expect(window.localStorage.getItem(providerIdStorageKey(WALLET_A))).toBe(PROVIDER_B);
  });

  it("forgets a field the operator deliberately cleared", async () => {
    // An emptied field is a statement; restoring the value over it would overrule the operator.
    await mount();
    type("providerId", PROVIDER_A);
    await turn();
    type("providerId", "");
    await turn();

    refresh();
    await mount();
    expect(providerIdField().value).toBe("");
  });
});

describe("two providers in one browser cannot read each other's id", () => {
  it("scopes the remembered id by wallet, on a refresh", async () => {
    await mount();
    type("providerId", PROVIDER_A);
    await turn();

    refresh();
    account.address = WALLET_B;
    await mount();
    // Wallet B has remembered nothing, so B sees no id - never A's.
    expect(providerIdField().value).toBe("");

    type("providerId", PROVIDER_B);
    await turn();
    refresh();
    account.address = WALLET_A;
    await mount();
    // Each wallet gets its own back.
    expect(providerIdField().value).toBe(PROVIDER_A);
  });

  it("switches the field with the wallet, mid-session", async () => {
    window.localStorage.setItem(providerIdStorageKey(WALLET_B), PROVIDER_B);
    await mount();
    type("providerId", PROVIDER_A);
    await turn();

    account.address = WALLET_B;
    root!.render(
      createElement(ProviderSelfServiceFlows, {
        contracts: CONTRACTS,
        missingConfig: [],
        capabilities: { issuance: true, listing: true },
      }),
    );
    await turn();
    await turn();
    expect(providerIdField().value).toBe(PROVIDER_B);
    // A's id is not lost by the switch - it is remembered under A, where switching back finds it.
    expect(window.localStorage.getItem(providerIdStorageKey(WALLET_A))).toBe(PROVIDER_A);
  });
});

describe("the memory itself", () => {
  const fake = () => {
    const map = new Map<string, string>();
    return {
      getItem: (k: string) => map.get(k) ?? null,
      setItem: (k: string, v: string) => void map.set(k, v),
      removeItem: (k: string) => void map.delete(k),
      map,
    };
  };

  it("keys by lowercased caller, so a checksummed and a lowercase wallet share one memory", () => {
    const storage = fake();
    rememberProviderId("0xAbCd000000000000000000000000000000000001", PROVIDER_A, storage);
    expect(
      recallProviderId("0xabcd000000000000000000000000000000000001", storage),
    ).toBe(PROVIDER_A);
    expect([...storage.map.keys()]).toEqual([
      `${PROVIDER_ID_STORAGE_PREFIX}0xabcd000000000000000000000000000000000001`,
    ]);
  });

  it("remembers nothing without a wallet - there is no scope to remember it under", () => {
    const storage = fake();
    rememberProviderId(undefined, PROVIDER_A, storage);
    expect(storage.map.size).toBe(0);
    expect(recallProviderId(undefined, storage)).toBeNull();
  });

  it("treats a blank remembered value as nothing rather than restoring whitespace", () => {
    const storage = fake();
    storage.map.set(providerIdStorageKey(WALLET_A), "   ");
    expect(recallProviderId(WALLET_A, storage)).toBeNull();
  });

  it("survives a storage that throws - remembering is a convenience, never a break", () => {
    const throwing = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
      removeItem: () => {
        throw new Error("blocked");
      },
    };
    expect(() => rememberProviderId(WALLET_A, PROVIDER_A, throwing)).not.toThrow();
    expect(recallProviderId(WALLET_A, throwing)).toBeNull();
  });
});
